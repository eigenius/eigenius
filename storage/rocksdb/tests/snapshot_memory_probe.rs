//! Diagnostic: measure RocksDB's in-RAM footprint for a loaded snapshot, per column family.
//!
//! Answers "does RocksDB accumulate memory with DB size under the default config?" — it opens the
//! snapshot with the SAME options `RocksStore::open` uses (all-defaults: max_open_files = -1, no
//! shared block cache, cache_index_and_filter_blocks = false) and dumps the memory-relevant
//! properties. `estimate-table-readers-mem` is the pinned index/filter-block RAM (grows with SST
//! count under the default config); `live-sst-files-size` sizes each CF on disk.
//!
//!   EIGENIUS_DB_SNAPSHOT=/path cargo test -p eigenius-storage-rocksdb --test snapshot_memory_probe \
//!       -- --ignored --nocapture

use rocksdb::{Options, DB};

const CFS: &[&str] = &["default", "cf_text", "cf_vec", "cf_embed_cache"];

fn mib(bytes: u64) -> String {
    format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
}

#[test]
#[ignore = "diagnostic: needs EIGENIUS_DB_SNAPSHOT; --ignored --nocapture"]
fn snapshot_memory_footprint_per_cf() {
    let Ok(path) = std::env::var("EIGENIUS_DB_SNAPSHOT") else {
        eprintln!("SKIP: set EIGENIUS_DB_SNAPSHOT to a loaded RocksDB store");
        return;
    };

    // Mirror RocksStore::open: defaults + Lz4, all four CFs. max_open_files stays -1 (default), so
    // opening pins every SST's table reader — the load-time steady state.
    let mut opts = Options::default();
    opts.create_if_missing(false);
    opts.set_compression_type(rocksdb::DBCompressionType::Lz4);
    let cf_opts: Vec<_> = CFS
        .iter()
        .map(|name| {
            let mut o = Options::default();
            o.set_compression_type(rocksdb::DBCompressionType::Lz4);
            rocksdb::ColumnFamilyDescriptor::new(*name, o)
        })
        .collect();

    let db = DB::open_cf_descriptors(&opts, &path, cf_opts).expect("open snapshot");

    // Global (DB-wide) memory properties.
    let g = |p: &str| db.property_int_value(p).ok().flatten().unwrap_or(0);
    eprintln!("\n=== DB-wide ===");
    eprintln!(
        "  block-cache-usage        {}",
        mib(g("rocksdb.block-cache-usage"))
    );
    eprintln!(
        "  cur-size-all-mem-tables  {}",
        mib(g("rocksdb.cur-size-all-mem-tables"))
    );
    eprintln!(
        "  estimate-table-readers-mem (DB) {}",
        mib(g("rocksdb.estimate-table-readers-mem"))
    );

    // Per-CF: pinned table-reader mem (index+filter blocks), on-disk size, key count.
    let mut total_readers = 0u64;
    let mut total_sst = 0u64;
    eprintln!("\n=== per column family ===");
    eprintln!(
        "  {:<16} {:>14} {:>16} {:>14}",
        "CF", "table-readers", "live-sst-size", "num-keys"
    );
    for cf_name in CFS {
        let cf = db.cf_handle(cf_name).expect("cf handle");
        let cfp = |p: &str| db.property_int_value_cf(&cf, p).ok().flatten().unwrap_or(0);
        let readers = cfp("rocksdb.estimate-table-readers-mem");
        let sst = cfp("rocksdb.live-sst-files-size");
        let keys = cfp("rocksdb.estimate-num-keys");
        total_readers += readers;
        total_sst += sst;
        eprintln!(
            "  {:<16} {:>14} {:>16} {:>14}",
            cf_name,
            mib(readers),
            mib(sst),
            keys
        );
    }
    eprintln!("\n  TOTAL table-readers-mem  {}", mib(total_readers));
    eprintln!("  TOTAL live-sst-size      {}", mib(total_sst));
    eprintln!(
        "\nInterpretation: table-readers-mem is the RAM RocksDB PINS for index+filter blocks under the\n\
         default config (cache_index_and_filter_blocks = false, max_open_files = -1). It grows with SST\n\
         count. If it is a small fraction of the {}, RocksDB internals are NOT the cumulative OOM driver.",
        mib(total_sst)
    );
}
