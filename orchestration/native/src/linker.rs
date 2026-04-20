//! Wasmtime Linker setup for the orchestrator IO world.
//!
//! Wires three host interfaces — io-access, read-access, query-access —
//! against an `HostBridge` trait object so the same linker works for both
//! napi-rs (production) and plain-closure (test) bridges.

use wasmtime::component::Linker;

use crate::host_state::HostState;

/// Build a Linker for the `eigenius-component-io` world.
pub fn build_io_linker(engine: &wasmtime::Engine) -> wasmtime::Result<Linker<HostState>> {
    let mut linker: Linker<HostState> = Linker::new(engine);
    link_io_access(&mut linker)?;
    link_read_access(&mut linker)?;
    link_query_access(&mut linker)?;
    Ok(linker)
}

fn link_io_access(linker: &mut Linker<HostState>) -> wasmtime::Result<()> {
    let mut iface = linker.instance("eigenius:component/io-access@0.1.0")?;
    iface.func_wrap_async(
        "dispatch-component",
        move |ctx, (iri, input, argument): (String, Vec<u8>, Vec<u8>)| {
            let bridge = ctx.data().bridge.clone();
            Box::new(async move {
                match bridge.dispatch(iri, input, argument).await {
                    Ok(bytes) => Ok((Ok::<Vec<u8>, String>(bytes),)),
                    Err(msg) => Ok((Err::<Vec<u8>, String>(msg),)),
                }
            })
        },
    )?;
    Ok(())
}

fn link_read_access(linker: &mut Linker<HostState>) -> wasmtime::Result<()> {
    let mut iface = linker.instance("eigenius:component/read-access@0.1.0")?;
    iface.func_wrap_async(
        "resolve",
        move |ctx, (iri,): (String,)| {
            let bridge = ctx.data().bridge.clone();
            Box::new(async move {
                let result = bridge
                    .resolve(iri)
                    .await
                    .map_err(wasmtime::Error::msg)?;
                Ok((result,))
            })
        },
    )?;
    Ok(())
}

fn link_query_access(linker: &mut Linker<HostState>) -> wasmtime::Result<()> {
    let mut iface = linker.instance("eigenius:component/query-access@0.1.0")?;
    iface.func_wrap_async(
        "query",
        move |ctx, (eigenql,): (String,)| {
            let bridge = ctx.data().bridge.clone();
            Box::new(async move {
                match bridge.query(eigenql).await {
                    Ok(rows) => Ok((Ok::<Vec<Vec<u8>>, String>(rows),)),
                    Err(msg) => Ok((Err::<Vec<Vec<u8>>, String>(msg),)),
                }
            })
        },
    )?;
    Ok(())
}
