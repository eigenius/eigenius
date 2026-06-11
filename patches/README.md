# patches/

Forks of third-party Cargo dependencies that we maintain in-tree because
the upstream version has a bug or missing feature we need.

Each subdirectory is a complete copy of the upstream crate at a pinned
version, with our local changes applied. The workspace `Cargo.toml`'s
`[patch.crates-io]` section redirects the published name to the local
path:

```toml
[patch.crates-io]
tonic-web = { path = "patches/tonic-web" }
```

The `deploy/Dockerfile.kernel` `COPY patches/ patches/` line keeps
the docker build context in sync — without it the container build
would resolve `path = "patches/tonic-web"` to a non-existent path
and fall back to the registry version.

## Current patches

### `tonic-web/` — upstream 0.14.6

Fixes a stuck-trailer bug in the gRPC-Web client decoder where, if a
hyper Data frame contains BOTH the gRPC-Web data frame and the gRPC-Web
trailer frame, the trailers are parsed and stored in
`GrpcWebCall.trailers` but never emitted to tonic. The end-of-stream
`Done(0)` branch in `poll_frame` returned `None` instead of flushing
stored trailers, so tonic saw a stream end without `grpc-status` and
failed every call with `"missing grpc-status trailer"`.

The trigger is reliable when the orchestrator side is Deno's HTTP/1.1
server with `Content-Length` (hyper buffers the whole 676-byte body
into one Data frame). The upstream code path that emits stored
trailers via the `Trailer(0)` branch only fires when the trailer
frame arrives in a separate hyper Data frame from the data — which
isn't deterministic across servers.

The patch is a four-line addition to the `Done(0)` branch:

```rust
FindTrailers::Done(len) => Poll::Ready(match len {
    0 => {
        // Flush any trailers parsed earlier but not yet emitted.
        if let Some(trailers) = me.as_mut().project().trailers.take() {
            Some(Ok(Frame::trailers(trailers)))
        } else {
            None
        }
    },
    _ => Some(Ok(Frame::data(buf.split_to(len).freeze()))),
}),
```

When the upstream releases a version with an equivalent fix, drop
this fork by removing the entry from `[patch.crates-io]` and the
`patches/tonic-web/` directory, and bumping the `tonic-web` minimum
version constraint in workspace dependencies.
