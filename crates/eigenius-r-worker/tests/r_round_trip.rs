// Copyright 2026 The Eigenius Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! P1.2 milestone: a real R↔Rust round-trip over a live Unix-domain
//! socket. `Rscript` runs the `EigeniusRWorker.R` driver, which loads the
//! built cdylib and drives the dispatch loop; this test plays the
//! substrate — sending requests and asserting responses. Proves R
//! receives a script over the wire, runs it, and returns the bytes
//! through the shared Rust transport.
//!
//! Skips gracefully (passes) when `Rscript` is unavailable, so the suite
//! is green on hosts without R.

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::thread::sleep;
use std::time::Duration;

use eigenius_runtime_substrate::rpc::codec::{decode_frame, encode_frame, MAX_FRAME_SIZE_DEFAULT};
use eigenius_runtime_substrate::rpc::protocol::{Request, Response, TargetKind};
use serde_bytes::ByteBuf;

/// `target/<profile>/libeigenius_r_worker.so`, derived from the test
/// binary's own path (`target/<profile>/deps/<test>`).
fn cdylib_path() -> PathBuf {
    let exe = std::env::current_exe().expect("test exe path");
    let profile_dir = exe
        .parent()
        .and_then(|deps| deps.parent())
        .expect("…/deps/.. = profile dir");
    let name = if cfg!(target_os = "macos") {
        "libeigenius_r_worker.dylib"
    } else {
        "libeigenius_r_worker.so"
    };
    profile_dir.join(name)
}

fn driver_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("r/EigeniusRWorker.R")
}

fn rscript_available() -> bool {
    Command::new("Rscript")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Connect to the worker's socket, retrying while `Rscript` cold-starts
/// and binds (R startup + `r_listen`'s accept can take a few seconds).
fn connect_with_retry(path: &std::path::Path) -> UnixStream {
    for _ in 0..200 {
        if let Ok(s) = UnixStream::connect(path) {
            return s;
        }
        sleep(Duration::from_millis(100));
    }
    panic!("could not connect to worker socket within 20s");
}

fn send(stream: &mut UnixStream, req: &Request) -> Response {
    encode_frame(req, stream).expect("encode request");
    stream.flush().expect("flush");
    decode_frame(stream, MAX_FRAME_SIZE_DEFAULT)
        .expect("decode response")
        .expect("a response frame (peer did not close)")
}

fn script_request(invocation_id: &str, source: &str) -> Request {
    let mut target = Vec::new();
    ciborium::into_writer(&source.to_string(), &mut target).expect("encode source");
    Request::DispatchMethod {
        invocation_id: invocation_id.to_string(),
        target_kind: TargetKind::Script,
        target: ByteBuf::from(target),
        inputs: vec![],
    }
}

struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn r_driver_round_trips_over_uds() {
    if !rscript_available() {
        eprintln!("skipping r_driver_round_trips_over_uds: Rscript not available");
        return;
    }
    let cdylib = cdylib_path();
    assert!(
        cdylib.exists(),
        "cdylib not built at {} (run `cargo build -p eigenius-r-worker`)",
        cdylib.display()
    );

    let tmp = tempfile::tempdir().expect("tempdir");
    let sock = tmp.path().join("worker.sock");

    // Spawn the R worker; it binds `sock` and blocks on accept.
    let child = Command::new("Rscript")
        .arg(driver_path())
        .arg(&sock)
        .arg(&cdylib)
        .spawn()
        .expect("spawn Rscript");
    let _guard = ChildGuard(child);

    let mut stream = connect_with_retry(&sock);

    // 1. Health.
    match send(&mut stream, &Request::Health) {
        Response::Health(_) => {}
        other => panic!("expected Health, got {other:?}"),
    }

    // 2. DispatchScript: R evaluates `as.raw(c(7,8,9))` → those bytes back.
    // The substrate opens a fresh connection per RPC, mirroring production;
    // the worker loops back to accept after each response... but P1.2's
    // driver reuses one connection, so continue on the same stream.
    match send(
        &mut stream,
        &script_request("inv-1", "as.raw(c(7L, 8L, 9L))"),
    ) {
        Response::DispatchOk {
            invocation_id,
            output,
            ..
        } => {
            assert_eq!(invocation_id, "inv-1");
            assert_eq!(&output[..], &[7u8, 8, 9]);
        }
        other => panic!("expected DispatchOk, got {other:?}"),
    }

    // 3. A script that computes — proves R actually ran it: sum(1:10) = 55.
    match send(&mut stream, &script_request("inv-2", "as.raw(sum(1:10))")) {
        Response::DispatchOk { output, .. } => assert_eq!(&output[..], &[55u8]),
        other => panic!("expected DispatchOk, got {other:?}"),
    }

    // 4. Evict → the driver responds and exits its loop.
    match send(&mut stream, &Request::Evict) {
        Response::Evicted => {}
        other => panic!("expected Evicted, got {other:?}"),
    }
}
