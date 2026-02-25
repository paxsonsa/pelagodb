use serde_json::Value;
use std::io::Read;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::OnceLock;
use std::thread::sleep;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const SERVER_READY_TIMEOUT: Duration = Duration::from_secs(30);
const SERVER_READY_POLL: Duration = Duration::from_millis(250);

pub struct CliTestFixture {
    pub server: String,
    pub database: String,
    pub namespace: String,
    server_child: Child,
}

impl CliTestFixture {
    pub fn start(test_name: &str) -> Self {
        let cluster_file = detect_cluster_file();
        let port = allocate_port();
        let listen_addr = format!("127.0.0.1:{}", port);
        let server = format!("http://{}", listen_addr);
        let token = unique_token(test_name);
        let database = format!("w5db_{}", token);
        let namespace = format!("w5ns_{}", token);
        let site_name = format!("w5-{}", token);
        let site_id = ((port % 200) + 20) as u8;

        let mut server_child = Command::new(server_bin())
            .arg("--listen-addr")
            .arg(&listen_addr)
            .arg("--site-id")
            .arg(site_id.to_string())
            .arg("--site-name")
            .arg(site_name)
            .arg("--fdb-cluster")
            .arg(cluster_file)
            .arg("--cache-enabled")
            .arg("false")
            .arg("--auth-required")
            .arg("false")
            .arg("--default-database")
            .arg(&database)
            .arg("--default-namespace")
            .arg(&namespace)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap_or_else(|e| panic!("failed to start pelago-server subprocess: {}", e));

        wait_for_server_ready(&mut server_child, &server, &database, &namespace);

        Self {
            server,
            database,
            namespace,
            server_child,
        }
    }

    pub fn run_ok(&self, args: &[&str]) -> Output {
        let output = self.run_raw(args, None);
        if output.status.success() {
            return output;
        }

        panic!(
            "pelago command failed: {}\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
            args.join(" "),
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }

    pub fn run_json(&self, args: &[&str]) -> Value {
        let mut with_format = vec!["-f", "json"];
        with_format.extend_from_slice(args);
        let output = self.run_ok(&with_format);
        let stdout = String::from_utf8_lossy(&output.stdout);
        serde_json::from_str(stdout.trim()).unwrap_or_else(|e| {
            panic!(
                "failed to parse json output for '{}': {}\nstdout:\n{}\nstderr:\n{}",
                with_format.join(" "),
                e,
                stdout,
                String::from_utf8_lossy(&output.stderr)
            )
        })
    }

    #[allow(dead_code)]
    pub fn run_repl_script(&self, script: &str) -> Output {
        let mut cmd = self.base_cli_command(&["repl"]);
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        let mut child = cmd
            .spawn()
            .unwrap_or_else(|e| panic!("failed to start pelago repl subprocess: {}", e));
        child
            .stdin
            .as_mut()
            .expect("missing repl stdin")
            .write_all(script.as_bytes())
            .expect("failed to write repl script");
        child.wait_with_output().expect("failed to wait on repl")
    }

    pub fn unique_entity_type(&self, base: &str) -> String {
        format!("{}_{}", base, unique_token("entity"))
    }

    fn run_raw(&self, args: &[&str], stdin_input: Option<&str>) -> Output {
        let mut cmd = self.base_cli_command(args);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        if let Some(stdin_input) = stdin_input {
            cmd.stdin(Stdio::piped());
            let mut child = cmd
                .spawn()
                .unwrap_or_else(|e| panic!("failed to spawn pelago command: {}", e));
            child
                .stdin
                .as_mut()
                .expect("missing command stdin")
                .write_all(stdin_input.as_bytes())
                .expect("failed to write command stdin");
            child
                .wait_with_output()
                .expect("failed to wait on pelago command")
        } else {
            cmd.output()
                .unwrap_or_else(|e| panic!("failed to execute pelago command: {}", e))
        }
    }

    fn base_cli_command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(pelago_bin());
        cmd.arg("--server")
            .arg(&self.server)
            .arg("--database")
            .arg(&self.database)
            .arg("--namespace")
            .arg(&self.namespace);
        cmd.args(args);
        cmd
    }
}

impl Drop for CliTestFixture {
    fn drop(&mut self) {
        if let Ok(None) = self.server_child.try_wait() {
            let _ = self.server_child.kill();
            let _ = self.server_child.wait();
        }
    }
}

fn wait_for_server_ready(child: &mut Child, server: &str, database: &str, namespace: &str) {
    let start = Instant::now();
    while start.elapsed() < SERVER_READY_TIMEOUT {
        if let Ok(Some(status)) = child.try_wait() {
            let (stdout, stderr) = drain_child_output(child);
            panic!(
                "pelago-server exited before becoming ready: {}\nstdout:\n{}\nstderr:\n{}",
                status, stdout, stderr
            );
        }

        let output = run_pelago_probe(server, database, namespace);
        if output.status.success() {
            return;
        }

        sleep(SERVER_READY_POLL);
    }

    let _ = child.kill();
    let _ = child.wait();
    let (stdout, stderr) = drain_child_output(child);
    panic!(
        "timed out waiting for pelago-server to become ready\nstdout:\n{}\nstderr:\n{}",
        stdout, stderr
    );
}

fn run_pelago_probe(server: &str, database: &str, namespace: &str) -> Output {
    Command::new(pelago_bin())
        .arg("--server")
        .arg(server)
        .arg("--database")
        .arg(database)
        .arg("--namespace")
        .arg(namespace)
        .arg("-f")
        .arg("json")
        .arg("schema")
        .arg("list")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap_or_else(|e| panic!("failed to run pelago readiness probe: {}", e))
}

fn drain_child_output(child: &mut Child) -> (String, String) {
    let mut stdout = String::new();
    let mut stderr = String::new();
    if let Some(mut out) = child.stdout.take() {
        let _ = out.read_to_string(&mut stdout);
    }
    if let Some(mut err) = child.stderr.take() {
        let _ = err.read_to_string(&mut stderr);
    }
    (stdout, stderr)
}

fn pelago_bin() -> &'static str {
    env!("CARGO_BIN_EXE_pelago")
}

fn server_bin() -> &'static Path {
    static SERVER_BIN: OnceLock<PathBuf> = OnceLock::new();
    SERVER_BIN
        .get_or_init(|| {
            if let Ok(bin) = std::env::var("PELAGO_SERVER_BIN") {
                let path = PathBuf::from(bin);
                if path.exists() {
                    return path;
                }
                panic!(
                    "PELAGO_SERVER_BIN is set but path does not exist: {}",
                    path.display()
                );
            }

            let root = workspace_root();
            let path = root.join("target").join("debug").join("pelago-server");
            if !path.exists() {
                let status = Command::new("cargo")
                    .arg("build")
                    .arg("-p")
                    .arg("pelago-server")
                    .current_dir(&root)
                    .status()
                    .unwrap_or_else(|e| panic!("failed to build pelago-server binary: {}", e));
                if !status.success() {
                    panic!("failed to build pelago-server binary: {}", status);
                }
            }
            path
        })
        .as_path()
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root not found")
        .to_path_buf()
}

fn detect_cluster_file() -> PathBuf {
    for key in ["PELAGO_FDB_CLUSTER", "FDB_CLUSTER_FILE"] {
        if let Ok(value) = std::env::var(key) {
            let path = PathBuf::from(value);
            if path.exists() {
                return path;
            }
        }
    }

    for candidate in [
        "/usr/local/etc/foundationdb/fdb.cluster",
        "/etc/foundationdb/fdb.cluster",
        "fdb.cluster",
    ] {
        let path = PathBuf::from(candidate);
        if path.exists() {
            return path;
        }
    }

    panic!(
        "FDB cluster file not found. Set FDB_CLUSTER_FILE or PELAGO_FDB_CLUSTER (for ignored CLI e2e tests)."
    );
}

fn allocate_port() -> u16 {
    let listener =
        std::net::TcpListener::bind("127.0.0.1:0").expect("failed to allocate local test port");
    listener
        .local_addr()
        .expect("failed to inspect allocated test port")
        .port()
}

fn unique_token(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_nanos();
    format!("{}_{}_{}", prefix, std::process::id(), nanos)
}
