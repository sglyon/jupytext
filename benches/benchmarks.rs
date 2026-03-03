//! Criterion benchmarks for jupytext-rs internal functions.
//!
//! Run with: cargo bench
//! Reports are generated in target/criterion/

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::collections::BTreeMap;
use std::fs;

use jupytext::cell_reader::{
    create_cell_reader, DoublePercentScriptCellReader, FormatOptions, LightScriptCellReader,
    MarkdownCellReader,
};
use jupytext::cell_to_text::{
    CellExporter, DoublePercentCellExporter, LightScriptCellExporter, MarkdownCellExporter,
};
use jupytext::formats::{divine_format, guess_format, long_form_one_format};
use jupytext::jupytext::{reads, writes};
use jupytext::notebook::{reads_ipynb, writes_ipynb, Cell, Notebook};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn bench_data_path(name: &str) -> String {
    format!(
        "{}/benches/data/{}.ipynb",
        env!("CARGO_MANIFEST_DIR"),
        name
    )
}

fn load_notebook(name: &str) -> (String, Notebook) {
    let path = bench_data_path(name);
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("Cannot read {}: {}", path, e));
    let nb = reads_ipynb(&text).unwrap();
    (text, nb)
}

fn notebook_to_percent(nb: &Notebook) -> String {
    writes(nb, "py:percent").unwrap()
}

fn notebook_to_markdown(nb: &Notebook) -> String {
    writes(nb, "md").unwrap()
}

// ---------------------------------------------------------------------------
// 1. ipynb parsing (JSON deserialization)
// ---------------------------------------------------------------------------

fn bench_parse_ipynb(c: &mut Criterion) {
    let mut group = c.benchmark_group("parse_ipynb");

    for name in &["small", "medium", "large", "xlarge"] {
        let (text, _nb) = load_notebook(name);
        group.throughput(Throughput::Bytes(text.len() as u64));
        group.bench_with_input(BenchmarkId::new("reads_ipynb", name), &text, |b, text| {
            b.iter(|| reads_ipynb(black_box(text)).unwrap());
        });
    }
    group.finish();
}

// ---------------------------------------------------------------------------
// 2. ipynb writing (JSON serialization)
// ---------------------------------------------------------------------------

fn bench_write_ipynb(c: &mut Criterion) {
    let mut group = c.benchmark_group("write_ipynb");

    for name in &["small", "medium", "large", "xlarge"] {
        let (_text, nb) = load_notebook(name);
        group.throughput(Throughput::Elements(nb.cells.len() as u64));
        group.bench_with_input(BenchmarkId::new("writes_ipynb", name), &nb, |b, nb| {
            b.iter(|| writes_ipynb(black_box(nb)).unwrap());
        });
    }
    group.finish();
}

// ---------------------------------------------------------------------------
// 3. Full conversion: ipynb -> py:percent (reads + writes)
// ---------------------------------------------------------------------------

fn bench_ipynb_to_percent(c: &mut Criterion) {
    let mut group = c.benchmark_group("ipynb_to_percent");

    for name in &["small", "medium", "large", "xlarge"] {
        let (text, _nb) = load_notebook(name);
        group.throughput(Throughput::Bytes(text.len() as u64));
        group.bench_with_input(
            BenchmarkId::new("full_conversion", name),
            &text,
            |b, text| {
                b.iter(|| {
                    let nb = reads_ipynb(black_box(text)).unwrap();
                    writes(&nb, "py:percent").unwrap()
                });
            },
        );
    }
    group.finish();
}

// ---------------------------------------------------------------------------
// 4. Full conversion: ipynb -> markdown
// ---------------------------------------------------------------------------

fn bench_ipynb_to_markdown(c: &mut Criterion) {
    let mut group = c.benchmark_group("ipynb_to_markdown");

    for name in &["small", "medium", "large", "xlarge"] {
        let (text, _nb) = load_notebook(name);
        group.throughput(Throughput::Bytes(text.len() as u64));
        group.bench_with_input(
            BenchmarkId::new("full_conversion", name),
            &text,
            |b, text| {
                b.iter(|| {
                    let nb = reads_ipynb(black_box(text)).unwrap();
                    writes(&nb, "md").unwrap()
                });
            },
        );
    }
    group.finish();
}

// ---------------------------------------------------------------------------
// 5. Reverse: py:percent -> ipynb
// ---------------------------------------------------------------------------

fn bench_percent_to_ipynb(c: &mut Criterion) {
    let mut group = c.benchmark_group("percent_to_ipynb");

    for name in &["small", "medium", "large", "xlarge"] {
        let (_text, nb) = load_notebook(name);
        let py_text = notebook_to_percent(&nb);
        group.throughput(Throughput::Bytes(py_text.len() as u64));
        group.bench_with_input(
            BenchmarkId::new("reads_percent", name),
            &py_text,
            |b, py_text| {
                b.iter(|| reads(black_box(py_text), Some("py:percent")).unwrap());
            },
        );
    }
    group.finish();
}

// ---------------------------------------------------------------------------
// 6. Reverse: markdown -> ipynb
// ---------------------------------------------------------------------------

fn bench_markdown_to_ipynb(c: &mut Criterion) {
    let mut group = c.benchmark_group("markdown_to_ipynb");

    for name in &["small", "medium", "large", "xlarge"] {
        let (_text, nb) = load_notebook(name);
        let md_text = notebook_to_markdown(&nb);
        group.throughput(Throughput::Bytes(md_text.len() as u64));
        group.bench_with_input(
            BenchmarkId::new("reads_markdown", name),
            &md_text,
            |b, md_text| {
                b.iter(|| reads(black_box(md_text), Some("md")).unwrap());
            },
        );
    }
    group.finish();
}

// ---------------------------------------------------------------------------
// 7. Round-trip: ipynb -> percent -> ipynb
// ---------------------------------------------------------------------------

fn bench_round_trip_percent(c: &mut Criterion) {
    let mut group = c.benchmark_group("round_trip_percent");

    for name in &["small", "medium", "large", "xlarge"] {
        let (text, _nb) = load_notebook(name);
        group.throughput(Throughput::Bytes(text.len() as u64));
        group.bench_with_input(
            BenchmarkId::new("ipynb_percent_ipynb", name),
            &text,
            |b, text| {
                b.iter(|| {
                    let nb = reads_ipynb(black_box(text)).unwrap();
                    let py = writes(&nb, "py:percent").unwrap();
                    reads(&py, Some("py:percent")).unwrap()
                });
            },
        );
    }
    group.finish();
}

// ---------------------------------------------------------------------------
// 8. Round-trip: ipynb -> markdown -> ipynb
// ---------------------------------------------------------------------------

fn bench_round_trip_markdown(c: &mut Criterion) {
    let mut group = c.benchmark_group("round_trip_markdown");

    for name in &["small", "medium", "large", "xlarge"] {
        let (text, _nb) = load_notebook(name);
        group.throughput(Throughput::Bytes(text.len() as u64));
        group.bench_with_input(
            BenchmarkId::new("ipynb_md_ipynb", name),
            &text,
            |b, text| {
                b.iter(|| {
                    let nb = reads_ipynb(black_box(text)).unwrap();
                    let md = writes(&nb, "md").unwrap();
                    reads(&md, Some("md")).unwrap()
                });
            },
        );
    }
    group.finish();
}

// ---------------------------------------------------------------------------
// 9. Format detection
// ---------------------------------------------------------------------------

fn bench_format_detection(c: &mut Criterion) {
    let mut group = c.benchmark_group("format_detection");

    for name in &["small", "medium", "large", "xlarge"] {
        let (text, nb) = load_notebook(name);
        let py_text = notebook_to_percent(&nb);
        let md_text = notebook_to_markdown(&nb);

        group.bench_with_input(
            BenchmarkId::new("divine_ipynb", name),
            &text,
            |b, text| {
                b.iter(|| divine_format(black_box(text)));
            },
        );
        group.bench_with_input(
            BenchmarkId::new("divine_percent", name),
            &py_text,
            |b, text| {
                b.iter(|| divine_format(black_box(text)));
            },
        );
        group.bench_with_input(
            BenchmarkId::new("guess_percent", name),
            &py_text,
            |b, text| {
                b.iter(|| guess_format(black_box(text), ".py"));
            },
        );
    }
    group.finish();
}

// ---------------------------------------------------------------------------
// 10. Cell reader micro-benchmarks
// ---------------------------------------------------------------------------

fn bench_cell_readers(c: &mut Criterion) {
    let mut group = c.benchmark_group("cell_readers");

    // Build a 50-cell percent script
    let (_text, nb) = load_notebook("large");
    let py_text = notebook_to_percent(&nb);
    let lines: Vec<String> = py_text.lines().map(|l| l.to_string()).collect();

    group.throughput(Throughput::Elements(lines.len() as u64));

    group.bench_function("percent_reader_100cell", |b| {
        let fmt = FormatOptions {
            extension: Some(".py".to_string()),
            format_name: Some("percent".to_string()),
            ..Default::default()
        };
        b.iter(|| {
            let mut pos = 0;
            let mut cells = 0;
            while pos < lines.len() {
                let mut reader = create_cell_reader(&fmt, Some("python"));
                let result = reader.read(&lines[pos..]);
                if result.next_position == 0 {
                    break;
                }
                pos += result.next_position;
                cells += 1;
            }
            cells
        });
    });

    // Build a 50-cell markdown notebook
    let md_text = notebook_to_markdown(&nb);
    let md_lines: Vec<String> = md_text.lines().map(|l| l.to_string()).collect();

    group.bench_function("markdown_reader_100cell", |b| {
        let fmt = FormatOptions {
            extension: Some(".md".to_string()),
            ..Default::default()
        };
        b.iter(|| {
            let mut pos = 0;
            let mut cells = 0;
            while pos < md_lines.len() {
                let mut reader = create_cell_reader(&fmt, Some("python"));
                let result = reader.read(&md_lines[pos..]);
                if result.next_position == 0 {
                    break;
                }
                pos += result.next_position;
                cells += 1;
            }
            cells
        });
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// 11. Cell exporter micro-benchmarks
// ---------------------------------------------------------------------------

fn bench_cell_exporters(c: &mut Criterion) {
    let mut group = c.benchmark_group("cell_exporters");

    let (_text, nb) = load_notebook("large");
    let fmt_py: BTreeMap<String, serde_json::Value> = {
        let mut m = BTreeMap::new();
        m.insert(
            "extension".to_string(),
            serde_json::Value::String(".py".to_string()),
        );
        m
    };
    let fmt_md: BTreeMap<String, serde_json::Value> = {
        let mut m = BTreeMap::new();
        m.insert(
            "extension".to_string(),
            serde_json::Value::String(".md".to_string()),
        );
        m
    };

    group.throughput(Throughput::Elements(nb.cells.len() as u64));

    group.bench_function("percent_exporter_100cell", |b| {
        b.iter(|| {
            let mut total_lines = 0;
            for cell in &nb.cells {
                let mut exp =
                    DoublePercentCellExporter::new(black_box(cell), "python", &fmt_py);
                total_lines += exp.cell_to_text().len();
            }
            total_lines
        });
    });

    group.bench_function("markdown_exporter_100cell", |b| {
        b.iter(|| {
            let mut total_lines = 0;
            for cell in &nb.cells {
                let mut exp =
                    MarkdownCellExporter::new(black_box(cell), "python", &fmt_md);
                total_lines += exp.cell_to_text().len();
            }
            total_lines
        });
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// 12. format parsing micro-benchmark
// ---------------------------------------------------------------------------

fn bench_format_parsing(c: &mut Criterion) {
    let mut group = c.benchmark_group("format_parsing");

    let formats = &[
        "py:percent",
        "md",
        "ipynb",
        "py:light",
        "R:spin",
        "notebooks///py:percent",
    ];

    for fmt in formats {
        group.bench_with_input(BenchmarkId::new("long_form", fmt), fmt, |b, fmt| {
            b.iter(|| long_form_one_format(black_box(fmt), None, None, false));
        });
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Criterion groups
// ---------------------------------------------------------------------------

criterion_group!(
    benches,
    bench_parse_ipynb,
    bench_write_ipynb,
    bench_ipynb_to_percent,
    bench_ipynb_to_markdown,
    bench_percent_to_ipynb,
    bench_markdown_to_ipynb,
    bench_round_trip_percent,
    bench_round_trip_markdown,
    bench_format_detection,
    bench_cell_readers,
    bench_cell_exporters,
    bench_format_parsing,
);

criterion_main!(benches);
