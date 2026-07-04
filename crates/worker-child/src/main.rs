fn main() -> std::process::ExitCode {
    // No subscriber is installed here: this binary's stdout is the worker protocol
    // channel, so a fmt layer must never touch it. The event is a no-op unless an
    // embedder installs a stderr subscriber in this process.
    tracing::debug!("worker-child starting stdio loop");
    lean_rs_worker_child::run_worker_child_stdio()
}
