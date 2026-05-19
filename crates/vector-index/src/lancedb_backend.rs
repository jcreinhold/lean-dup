use std::cmp::Ordering;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use arrow_array::cast::AsArray;
use arrow_array::types::Float32Type;
use arrow_array::{FixedSizeListArray, Float32Array, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema};
use futures::TryStreamExt;
use lancedb::index::Index;
use lancedb::query::{ExecutableQuery, QueryBase};
use lancedb::{DistanceType, connect};
use serde::{Deserialize, Serialize};

use crate::{
    Result, VECTOR_INDEX_SCHEMA_VERSION, VectorCorpus, VectorCorpusBuildCounters, VectorCorpusBuildOutput,
    VectorCorpusBuildRequest, VectorCorpusOpenRequest, VectorCorpusProvenance, VectorCorpusQueryCounters,
    VectorCorpusQueryOutput, VectorCorpusQueryRequest, VectorCorpusStatus, VectorCorpusSummary, VectorDeclaration,
    VectorIndexError, VectorNearestDeclaration,
};

const MANIFEST_FILE: &str = "corpus-manifest.json";
const DATABASE_DIR: &str = "corpus";
const DECLARATION_TABLE: &str = "declarations";
const COLUMN_DECLARATION_ID: &str = "declaration_id";
const COLUMN_DECLARATION_NAME: &str = "declaration_name";
const COLUMN_MODULE_NAME: &str = "module_name";
const COLUMN_DECLARATION_KIND: &str = "declaration_kind";
const COLUMN_CONTENT_HASH: &str = "content_hash";
const COLUMN_VECTOR: &str = "vector";
const COLUMN_DISTANCE: &str = "_distance";
const MIN_ROWS_FOR_BACKEND_INDEX: usize = 256;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct Manifest {
    schema_version: String,
    provenance: VectorCorpusProvenance,
    declaration_count: usize,
    vector_dimension: usize,
}

pub(crate) fn build_vector_corpus(request: VectorCorpusBuildRequest) -> Result<VectorCorpusBuildOutput> {
    let start = Instant::now();
    let previous_status = current_status(&request.cache_root, &request.provenance);
    if previous_status == VectorCorpusStatus::Reused {
        let manifest = read_manifest(&request.cache_root)?;
        return Ok(VectorCorpusBuildOutput {
            summary: summary(VectorCorpusStatus::Reused, manifest),
            previous_status,
            counters: VectorCorpusBuildCounters {
                input_declarations: request.declarations.len(),
                stored_declarations: 0,
                build_ms: start.elapsed().as_millis(),
            },
        });
    }

    rebuild_corpus(&request)?;
    let manifest = Manifest {
        schema_version: VECTOR_INDEX_SCHEMA_VERSION.to_owned(),
        provenance: request.provenance,
        declaration_count: request.declarations.len(),
        vector_dimension: request
            .declarations
            .first()
            .map_or(0, |declaration| declaration.vector.len()),
    };
    write_manifest(&request.cache_root, &manifest)?;

    Ok(VectorCorpusBuildOutput {
        summary: summary(VectorCorpusStatus::Built, manifest),
        previous_status,
        counters: VectorCorpusBuildCounters {
            input_declarations: request.declarations.len(),
            stored_declarations: request.declarations.len(),
            build_ms: start.elapsed().as_millis(),
        },
    })
}

pub(crate) fn open_vector_corpus(request: VectorCorpusOpenRequest) -> Result<VectorCorpus> {
    let manifest = read_manifest(&request.cache_root).map_err(|error| match error {
        VectorIndexError::Io { .. } => {
            VectorIndexError::unavailable(VectorCorpusStatus::Missing, "persisted vector corpus is missing")
        }
        other @ (VectorIndexError::InvalidRequest { .. }
        | VectorIndexError::CorpusUnavailable { .. }
        | VectorIndexError::Storage { .. }
        | VectorIndexError::Manifest { .. }) => other,
    })?;
    if manifest.schema_version != VECTOR_INDEX_SCHEMA_VERSION || manifest.provenance != request.provenance {
        return Err(VectorIndexError::unavailable(
            VectorCorpusStatus::Stale,
            "persisted vector corpus provenance does not match request",
        ));
    }
    block_on(async {
        let database = connect_database(&request.cache_root).await?;
        database
            .open_table(DECLARATION_TABLE)
            .execute()
            .await
            .map_err(|source| VectorIndexError::unavailable(VectorCorpusStatus::Unusable, source.to_string()))?;
        Ok(())
    })?;
    Ok(VectorCorpus::new(
        request.cache_root,
        summary(VectorCorpusStatus::Reused, manifest),
    ))
}

pub(crate) fn query_vector_corpus(
    corpus: &VectorCorpus,
    request: &VectorCorpusQueryRequest,
) -> Result<VectorCorpusQueryOutput> {
    let start = Instant::now();
    let nearest = block_on(async {
        let database = connect_database(&corpus.cache_root).await?;
        let table = database
            .open_table(DECLARATION_TABLE)
            .execute()
            .await
            .map_err(|source| VectorIndexError::unavailable(VectorCorpusStatus::Unusable, source.to_string()))?;
        let mut batches = table
            .query()
            .nearest_to(request.query_vector.as_slice())
            .map_err(|source| VectorIndexError::storage(source.to_string()))?
            .distance_type(DistanceType::Cosine)
            .limit(request.limit)
            .execute()
            .await
            .map_err(|source| VectorIndexError::storage(source.to_string()))?;

        let mut nearest = Vec::new();
        while let Some(batch) = batches
            .try_next()
            .await
            .map_err(|source| VectorIndexError::storage(source.to_string()))?
        {
            nearest.extend(nearest_from_batch(&batch)?);
        }
        nearest.sort_by(compare_nearest);
        nearest.truncate(request.limit);
        Ok(nearest)
    })?;

    Ok(VectorCorpusQueryOutput {
        summary: corpus.summary.clone(),
        counters: VectorCorpusQueryCounters {
            requested_limit: request.limit,
            returned: nearest.len(),
            query_ms: start.elapsed().as_millis(),
        },
        nearest,
    })
}

fn current_status(cache_root: &Path, requested: &VectorCorpusProvenance) -> VectorCorpusStatus {
    match read_manifest(cache_root) {
        Ok(manifest) => {
            if manifest.schema_version == VECTOR_INDEX_SCHEMA_VERSION && manifest.provenance == *requested {
                VectorCorpusStatus::Reused
            } else {
                VectorCorpusStatus::Stale
            }
        }
        Err(VectorIndexError::Io { .. }) => VectorCorpusStatus::Missing,
        Err(_) => VectorCorpusStatus::Unusable,
    }
}

fn rebuild_corpus(request: &VectorCorpusBuildRequest) -> Result<()> {
    fs::create_dir_all(&request.cache_root)?;
    let database_path = database_path(&request.cache_root);
    if database_path.exists() {
        fs::remove_dir_all(&database_path)?;
    }
    block_on(async {
        let database = connect_database(&request.cache_root).await?;
        let batch = declarations_batch(&request.declarations, request.provenance.vector_dimension)?;
        let table = database
            .create_table(DECLARATION_TABLE, batch)
            .execute()
            .await
            .map_err(|source| VectorIndexError::storage(source.to_string()))?;
        if request.declarations.len() >= MIN_ROWS_FOR_BACKEND_INDEX {
            table
                .create_index(&[COLUMN_VECTOR], Index::Auto)
                .execute()
                .await
                .map_err(|source| VectorIndexError::storage(source.to_string()))?;
        }
        Ok(())
    })
}

async fn connect_database(cache_root: &Path) -> Result<lancedb::Connection> {
    let database_path = database_path(cache_root);
    let database_uri = database_path
        .to_str()
        .ok_or_else(|| VectorIndexError::storage("vector corpus cache path must be UTF-8"))?;
    connect(database_uri)
        .execute()
        .await
        .map_err(|source| VectorIndexError::storage(source.to_string()))
}

fn declarations_batch(declarations: &[VectorDeclaration], dimension: usize) -> Result<RecordBatch> {
    let schema = Arc::new(Schema::new(vec![
        Field::new(COLUMN_DECLARATION_ID, DataType::Utf8, false),
        Field::new(COLUMN_DECLARATION_NAME, DataType::Utf8, false),
        Field::new(COLUMN_MODULE_NAME, DataType::Utf8, false),
        Field::new(COLUMN_DECLARATION_KIND, DataType::Utf8, false),
        Field::new(COLUMN_CONTENT_HASH, DataType::Utf8, false),
        Field::new(
            COLUMN_VECTOR,
            DataType::FixedSizeList(
                Arc::new(Field::new("item", DataType::Float32, true)),
                i32::try_from(dimension)
                    .map_err(|_| VectorIndexError::invalid("vector dimension exceeds supported size"))?,
            ),
            false,
        ),
    ]));
    let vectors = FixedSizeListArray::from_iter_primitive::<Float32Type, _, _>(
        declarations
            .iter()
            .map(|declaration| Some(declaration.vector.iter().copied().map(Some).collect::<Vec<_>>())),
        i32::try_from(dimension).map_err(|_| VectorIndexError::invalid("vector dimension exceeds supported size"))?,
    );
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from_iter_values(
                declarations
                    .iter()
                    .map(|declaration| declaration.declaration_id.as_str()),
            )),
            Arc::new(StringArray::from_iter_values(
                declarations
                    .iter()
                    .map(|declaration| declaration.declaration_name.as_str()),
            )),
            Arc::new(StringArray::from_iter_values(
                declarations.iter().map(|declaration| declaration.module_name.as_str()),
            )),
            Arc::new(StringArray::from_iter_values(
                declarations
                    .iter()
                    .map(|declaration| declaration.declaration_kind.as_str()),
            )),
            Arc::new(StringArray::from_iter_values(
                declarations.iter().map(|declaration| declaration.content_hash.as_str()),
            )),
            Arc::new(vectors),
        ],
    )
    .map_err(|source| VectorIndexError::storage(source.to_string()))
}

fn nearest_from_batch(batch: &RecordBatch) -> Result<Vec<VectorNearestDeclaration>> {
    let ids = string_column(batch, COLUMN_DECLARATION_ID)?;
    let names = string_column(batch, COLUMN_DECLARATION_NAME)?;
    let modules = string_column(batch, COLUMN_MODULE_NAME)?;
    let kinds = string_column(batch, COLUMN_DECLARATION_KIND)?;
    let hashes = string_column(batch, COLUMN_CONTENT_HASH)?;
    let distances = distance_column(batch)?;
    let mut nearest = Vec::with_capacity(batch.num_rows());
    for row in 0..batch.num_rows() {
        nearest.push(VectorNearestDeclaration {
            declaration_id: ids.value(row).to_owned(),
            declaration_name: names.value(row).to_owned(),
            module_name: modules.value(row).to_owned(),
            declaration_kind: kinds.value(row).to_owned(),
            content_hash: hashes.value(row).to_owned(),
            score: cosine_distance_to_score(distances.value(row)),
        });
    }
    Ok(nearest)
}

fn string_column<'a>(batch: &'a RecordBatch, column: &str) -> Result<&'a StringArray> {
    batch
        .column_by_name(column)
        .ok_or_else(|| VectorIndexError::storage(format!("vector corpus result missing {column}")))?
        .as_string_opt::<i32>()
        .ok_or_else(|| VectorIndexError::storage(format!("vector corpus result has invalid {column} column")))
}

fn distance_column(batch: &RecordBatch) -> Result<&Float32Array> {
    batch
        .column_by_name(COLUMN_DISTANCE)
        .ok_or_else(|| VectorIndexError::storage("vector corpus result missing score column"))?
        .as_primitive_opt::<Float32Type>()
        .ok_or_else(|| VectorIndexError::storage("vector corpus result has invalid score column"))
}

fn cosine_distance_to_score(distance: f32) -> f32 {
    1.0 - distance
}

fn compare_nearest(left: &VectorNearestDeclaration, right: &VectorNearestDeclaration) -> Ordering {
    right
        .score
        .total_cmp(&left.score)
        .then_with(|| left.declaration_name.cmp(&right.declaration_name))
        .then_with(|| left.declaration_id.cmp(&right.declaration_id))
}

fn read_manifest(cache_root: &Path) -> Result<Manifest> {
    let contents = fs::read_to_string(manifest_path(cache_root))?;
    serde_json::from_str(&contents).map_err(Into::into)
}

fn write_manifest(cache_root: &Path, manifest: &Manifest) -> Result<()> {
    fs::create_dir_all(cache_root)?;
    let contents = serde_json::to_string_pretty(manifest)?;
    fs::write(manifest_path(cache_root), contents)?;
    Ok(())
}

fn manifest_path(cache_root: &Path) -> PathBuf {
    cache_root.join(MANIFEST_FILE)
}

fn database_path(cache_root: &Path) -> PathBuf {
    cache_root.join(DATABASE_DIR)
}

fn summary(status: VectorCorpusStatus, manifest: Manifest) -> VectorCorpusSummary {
    VectorCorpusSummary {
        schema_version: manifest.schema_version,
        status,
        provenance: manifest.provenance,
        declaration_count: manifest.declaration_count,
        vector_dimension: manifest.vector_dimension,
    }
}

fn block_on<T>(future: impl std::future::Future<Output = Result<T>>) -> Result<T> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|source| VectorIndexError::storage(source.to_string()))?
        .block_on(future)
}
