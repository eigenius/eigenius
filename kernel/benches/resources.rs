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

//! Resource construction and property lookup.
//!
//! Group `chunk` runs the paths a resource's property map sits on over a real lexicon chunk: the
//! ESL compiler building resources from an already-parsed file, the CBOR codec writing and reading
//! them, a clone of each, and lookups. Group `lookup` gets every key of one resource with n
//! properties. Group `search` runs a binary search and two linear scans over one sorted slice, for
//! the n where they trade places.
//!
//! `chunk` and `lookup` are written against `Resource`'s API alone, so they run unchanged on any
//! representation of the map, which is what lets one commit be compared with another.
//!
//! The chunk is `wordnet-chain/wordnet-003.esl` at the workspace root, which the lexicon import
//! generates and git ignores; `EIGENIUS_BENCH_ESL` names another file.

use std::cmp::Ordering;
use std::hint::black_box;
use std::path::PathBuf;
use std::time::Duration;

use criterion::measurement::WallTime;
use criterion::{
    criterion_group, criterion_main, BatchSize, BenchmarkGroup, Criterion, Throughput,
};
use eigenius_kernel::bootstrap::bootstrap;
use eigenius_kernel::esl::{self, compile, lexer, parser};
use eigenius_kernel::ontology::eigon_cbor::{parse_resource, serialize_resource};
use eigenius_kernel::ontology::well_known as wk;
use eigenius_kernel::ontology::{Iri, Resource, Value};
use eigenius_kernel::units::convert::Vocabulary;

fn chunk_path() -> PathBuf {
    std::env::var_os("EIGENIUS_BENCH_ESL")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../wordnet-chain/wordnet-003.esl")
        })
}

/// Every resource node under `r`, `r` included, depth first.
fn nodes<'a>(r: &'a Resource, out: &mut Vec<&'a Resource>) {
    out.push(r);
    for (_, v) in r.properties() {
        embedded(v, out);
    }
}

fn embedded<'a>(v: &'a Value, out: &mut Vec<&'a Resource>) {
    match v {
        Value::Embedded(r) => nodes(r, out),
        Value::Array(vs) => vs.iter().for_each(|v| embedded(v, out)),
        _ => {}
    }
}

/// How many properties the chunk's nodes hold, printed once so a lookup threshold can be read
/// against it.
fn print_sizes(all: &[&Resource], entries: usize) {
    let bounds = [1, 2, 3, 4, 8, 16, usize::MAX];
    let mut counts = [0usize; 7];
    for r in all {
        let n = r.properties().len();
        counts[bounds
            .iter()
            .position(|&b| n <= b)
            .expect("MAX bounds every n")] += 1;
    }
    let keys: usize = all.iter().map(|r| r.properties().len()).sum();
    let max = all.iter().map(|r| r.properties().len()).max().unwrap_or(0);
    eprintln!(
        "chunk: {entries} resources, {} nodes, {keys} property keys, max {max} on one node",
        all.len()
    );
    eprintln!(
        "properties per node: <=1 {}, 2 {}, 3 {}, 4 {}, 5-8 {}, 9-16 {}, >16 {}",
        counts[0], counts[1], counts[2], counts[3], counts[4], counts[5], counts[6]
    );
}

fn chunk(c: &mut Criterion) {
    let path = chunk_path();
    let source = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "{}: {e} (EIGENIUS_BENCH_ESL names the chunk)",
            path.display()
        )
    });
    let ctx = bootstrap().expect("bootstrap builds");
    let layer = ctx.head();
    let tokens = lexer::tokenize(&source).expect("chunk lexes");
    let file = parser::parse(&tokens).expect("chunk parses");
    let ctors = compile::collect_ctors_from_layer(layer);
    let macros = compile::collect_macros_from_layer(layer);
    let units = Vocabulary::from_layer(layer).ok();
    let resources = esl::compile(&source, layer).expect("chunk compiles");

    let mut all = Vec::new();
    resources.iter().for_each(|r| nodes(r, &mut all));
    print_sizes(&all, resources.len());

    let cbor: Vec<Vec<u8>> = resources.iter().map(serialize_resource).collect();
    // Owned copies, as a caller holds them: a key borrowed from the map it is looked up in would
    // compare against its own bytes.
    let own_keys: Vec<Vec<Iri>> = all
        .iter()
        .map(|r| r.property_iris().cloned().collect())
        .collect();
    let well_known: Vec<Iri> = [
        wk::IS_A,
        wk::SHORT_NAME,
        wk::DESCRIPTION,
        wk::TYPE_NAME,
        wk::TYPE_ARGS,
    ]
    .iter()
    .map(|s| Iri::parse(s).expect("well-known IRI parses"))
    .collect();

    let mut g = c.benchmark_group("chunk");
    g.sample_size(10).measurement_time(Duration::from_secs(20));
    g.bench_function("esl_compile", |b| {
        b.iter_batched(
            || (ctors.clone(), macros.clone(), units.clone()),
            |(ctors, macros, units)| {
                compile::compile_file_with_context(&file, None, ctors, macros, units)
                    .expect("chunk compiles")
            },
            BatchSize::LargeInput,
        )
    });
    g.bench_function("cbor_encode", |b| {
        b.iter(|| resources.iter().map(serialize_resource).collect::<Vec<_>>())
    });
    g.bench_function("cbor_decode", |b| {
        b.iter(|| {
            cbor.iter()
                .map(|bytes| parse_resource(bytes).expect("round trip"))
                .collect::<Vec<_>>()
        })
    });
    g.bench_function("clone", |b| b.iter(|| resources.clone()));
    g.bench_function("get_own_keys", |b| {
        b.iter(|| {
            let mut hits = 0usize;
            for (r, keys) in all.iter().zip(&own_keys) {
                for k in keys {
                    hits += r.get(black_box(k)).is_some() as usize;
                }
            }
            hits
        })
    });
    // The keys validation and indexing ask every node for, present or not.
    g.bench_function("get_well_known", |b| {
        b.iter(|| {
            let mut hits = 0usize;
            for r in &all {
                for k in &well_known {
                    hits += r.get(black_box(k)).is_some() as usize;
                }
            }
            hits
        })
    });
    g.finish();
}

fn lookup(c: &mut Criterion) {
    let mut g = c.benchmark_group("lookup");
    for n in [2usize, 3, 4, 6, 8, 12, 16, 24, 32] {
        let keys: Vec<Iri> = (0..n)
            .map(|i| Iri::parse(&format!("urn:eigenius:lexicon:property_{i:02}")).expect("IRI"))
            .collect();
        let mut r = Resource::new_embedded();
        for k in &keys {
            r.set(k.clone(), Value::Integer(0));
        }
        let missing = Iri::parse("urn:eigenius:lexicon:property_xx").expect("IRI");
        g.bench_function(format!("hit/{n}"), |b| {
            b.iter(|| {
                keys.iter()
                    .filter(|k| r.get(black_box(k)).is_some())
                    .count()
            })
        });
        g.bench_function(format!("miss/{n}"), |b| {
            b.iter(|| r.get(black_box(&missing)).is_some())
        });
    }
    g.finish();
}

/// A binary search of a sorted slice.
fn binary(entries: &[(Iri, Value)], key: &Iri) -> Option<usize> {
    entries.binary_search_by(|(k, _)| k.cmp(key)).ok()
}

/// A forward scan that stops at the first key not below the probe, as `BTreeMap` searches a node.
fn linear_cmp(entries: &[(Iri, Value)], key: &Iri) -> Option<usize> {
    for (i, (k, _)) in entries.iter().enumerate() {
        match k.cmp(key) {
            Ordering::Less => {}
            Ordering::Equal => return Some(i),
            Ordering::Greater => return None,
        }
    }
    None
}

/// A forward scan for an equal key, which compares lengths before bytes and never stops early.
fn linear_eq(entries: &[(Iri, Value)], key: &Iri) -> Option<usize> {
    entries.iter().position(|(k, _)| k == key)
}

fn bench_find(
    g: &mut BenchmarkGroup<'_, WallTime>,
    name: &str,
    entries: &[(Iri, Value)],
    hits: &[Iri],
    misses: &[Iri],
    find: impl Fn(&[(Iri, Value)], &Iri) -> Option<usize>,
) {
    let n = entries.len();
    g.bench_function(format!("{name}/hit/{n}"), |b| {
        b.iter(|| {
            hits.iter()
                .filter(|k| find(black_box(entries), black_box(k)).is_some())
                .count()
        })
    });
    g.bench_function(format!("{name}/miss/{n}"), |b| {
        b.iter(|| {
            misses
                .iter()
                .filter(|k| find(black_box(entries), black_box(k)).is_some())
                .count()
        })
    });
}

/// The three searches on one sorted `(Iri, Value)` slice of n entries, each key looked up once.
/// Keys and misses have one length, so a length check rejects nothing; each miss falls in a
/// different gap between keys.
fn search(c: &mut Criterion) {
    let mut g = c.benchmark_group("search");
    g.warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(2));
    for n in [2usize, 3, 4, 6, 8, 12, 16, 24, 32, 48, 64, 96, 128] {
        let key =
            |i: usize| Iri::parse(&format!("urn:eigenius:lexicon:property_{i:04}")).expect("IRI");
        let entries: Vec<(Iri, Value)> = (0..n).map(|i| (key(2 * i), Value::Integer(0))).collect();
        let hits: Vec<Iri> = (0..n).map(|i| key(2 * i)).collect();
        let misses: Vec<Iri> = (0..n).map(|i| key(2 * i + 1)).collect();
        g.throughput(Throughput::Elements(n as u64));
        bench_find(&mut g, "binary", &entries, &hits, &misses, binary);
        bench_find(&mut g, "linear_cmp", &entries, &hits, &misses, linear_cmp);
        bench_find(&mut g, "linear_eq", &entries, &hits, &misses, linear_eq);
    }
    g.finish();
}

criterion_group!(benches, chunk, lookup, search);
criterion_main!(benches);
