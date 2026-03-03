#!/usr/bin/env bash
# ============================================================================
# Benchmark: Python jupytext vs Rust jupytext-rs
#
# Requires:
#   - hyperfine  (brew install hyperfine)
#   - jupytext   (pip install jupytext)
#   - Rust binary built in release mode
#
# Usage:
#   ./benches/compare.sh              # run all benchmarks
#   ./benches/compare.sh --quick      # fewer iterations, faster
#   ./benches/compare.sh --export     # also export JSON results
# ============================================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
DATA_DIR="$SCRIPT_DIR/data"
RESULTS_DIR="$SCRIPT_DIR/results"

RUST_BIN="$ROOT_DIR/target/release/jupytext"
PY_BIN="$(command -v jupytext)"

# Parse flags
QUICK=false
EXPORT=false
for arg in "$@"; do
    case $arg in
        --quick) QUICK=true ;;
        --export) EXPORT=true ;;
    esac
done

# ---------------------------------------------------------------------------
# Preflight checks
# ---------------------------------------------------------------------------
echo "=== Preflight ==="
if [[ ! -x "$RUST_BIN" ]]; then
    echo "Building Rust release binary..."
    (cd "$ROOT_DIR" && cargo build --release --quiet)
fi
echo "  Rust:   $RUST_BIN  ($($RUST_BIN --version 2>&1 || echo 'v?'))"
echo "  Python: $PY_BIN  ($($PY_BIN --version 2>&1 || echo 'v?'))"
echo "  hyperfine: $(hyperfine --version)"
echo ""

# Generate benchmark data if missing
if [[ ! -f "$DATA_DIR/small.ipynb" ]]; then
    echo "Generating benchmark notebooks..."
    python3 "$SCRIPT_DIR/gen_notebooks.py" "$DATA_DIR"
    echo ""
fi

# Create results and tmp dirs
mkdir -p "$RESULTS_DIR"
TMP_DIR=$(mktemp -d)
trap "rm -rf $TMP_DIR" EXIT

# ---------------------------------------------------------------------------
# Configure hyperfine
# ---------------------------------------------------------------------------
if $QUICK; then
    MIN_RUNS=3
    WARMUP=1
else
    MIN_RUNS=10
    WARMUP=3
fi

COMMON_ARGS=(--warmup "$WARMUP" --min-runs "$MIN_RUNS")
if $EXPORT; then
    EXPORT_FLAG=(--export-json)
else
    EXPORT_FLAG=()
fi

run_bench() {
    local label="$1"
    shift
    echo ""
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "  $label"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

    local extra=()
    if [[ ${#EXPORT_FLAG[@]} -gt 0 ]]; then
        local safe_name
        safe_name=$(echo "$label" | tr ' /:' '___')
        extra=("${EXPORT_FLAG[@]}" "$RESULTS_DIR/${safe_name}.json")
    fi

    hyperfine "${COMMON_ARGS[@]}" "${extra[@]}" "$@"
}

# ---------------------------------------------------------------------------
# 1. ipynb -> py:percent
# ---------------------------------------------------------------------------
for size in small medium large xlarge; do
    NB="$DATA_DIR/${size}.ipynb"
    [[ -f "$NB" ]] || continue
    BYTES=$(wc -c < "$NB" | tr -d ' ')

    run_bench "ipynb -> py:percent  [${size}, ${BYTES} bytes]" \
        --command-name "rust" \
        "'$RUST_BIN' --from ipynb --to py:percent '$NB' --output '$TMP_DIR/out.py'" \
        --command-name "python" \
        "'$PY_BIN' --from ipynb --to py:percent '$NB' --output '$TMP_DIR/out_py.py'"
done

# ---------------------------------------------------------------------------
# 2. ipynb -> markdown
# ---------------------------------------------------------------------------
for size in small medium large xlarge; do
    NB="$DATA_DIR/${size}.ipynb"
    [[ -f "$NB" ]] || continue
    BYTES=$(wc -c < "$NB" | tr -d ' ')

    run_bench "ipynb -> markdown  [${size}, ${BYTES} bytes]" \
        --command-name "rust" \
        "'$RUST_BIN' --from ipynb --to md '$NB' --output '$TMP_DIR/out.md'" \
        --command-name "python" \
        "'$PY_BIN' --from ipynb --to md '$NB' --output '$TMP_DIR/out_py.md'"
done

# ---------------------------------------------------------------------------
# 3. py:percent -> ipynb  (reverse direction)
# ---------------------------------------------------------------------------
# First, generate the .py files
for size in small medium large xlarge; do
    NB="$DATA_DIR/${size}.ipynb"
    PY="$DATA_DIR/${size}.py"
    [[ -f "$NB" ]] || continue
    if [[ ! -f "$PY" ]]; then
        "$RUST_BIN" --from ipynb --to py:percent "$NB" --output "$PY" 2>/dev/null || true
    fi
done

for size in small medium large xlarge; do
    PY="$DATA_DIR/${size}.py"
    [[ -f "$PY" ]] || continue
    BYTES=$(wc -c < "$PY" | tr -d ' ')

    run_bench "py:percent -> ipynb  [${size}, ${BYTES} bytes]" \
        --command-name "rust" \
        "'$RUST_BIN' --from py:percent --to ipynb '$PY' --output '$TMP_DIR/out.ipynb'" \
        --command-name "python" \
        "'$PY_BIN' --from py:percent --to ipynb '$PY' --output '$TMP_DIR/out_py.ipynb'"
done

# ---------------------------------------------------------------------------
# 4. markdown -> ipynb  (reverse direction)
# ---------------------------------------------------------------------------
for size in small medium large xlarge; do
    NB="$DATA_DIR/${size}.ipynb"
    MD="$DATA_DIR/${size}.md"
    [[ -f "$NB" ]] || continue
    if [[ ! -f "$MD" ]]; then
        "$RUST_BIN" --from ipynb --to md "$NB" --output "$MD" 2>/dev/null || true
    fi
done

for size in small medium large xlarge; do
    MD="$DATA_DIR/${size}.md"
    [[ -f "$MD" ]] || continue
    BYTES=$(wc -c < "$MD" | tr -d ' ')

    run_bench "markdown -> ipynb  [${size}, ${BYTES} bytes]" \
        --command-name "rust" \
        "'$RUST_BIN' --from md --to ipynb '$MD' --output '$TMP_DIR/out2.ipynb'" \
        --command-name "python" \
        "'$PY_BIN' --from md --to ipynb '$MD' --output '$TMP_DIR/out2_py.ipynb'"
done

# ---------------------------------------------------------------------------
# 5. Round-trip: ipynb -> percent -> ipynb (piped)
# ---------------------------------------------------------------------------
for size in small medium large xlarge; do
    NB="$DATA_DIR/${size}.ipynb"
    [[ -f "$NB" ]] || continue

    # For the round-trip we chain: read ipynb, write percent, then read percent back.
    # We use --test flag which does an internal round-trip check.
    run_bench "round-trip --test  [${size}]" \
        --command-name "rust" \
        "'$RUST_BIN' --from ipynb --to py:percent --test '$NB'" \
        --command-name "python" \
        "'$PY_BIN' --from ipynb --to py:percent --test '$NB'"
done

# ---------------------------------------------------------------------------
# 6. Real-world notebooks from test suite
# ---------------------------------------------------------------------------
REAL_NB_DIR="$ROOT_DIR/tests/data/notebooks/inputs/ipynb_py"
if [[ -d "$REAL_NB_DIR" ]]; then
    # Pick a few representative ones
    for nb_name in jupyter.ipynb "Notebook with metadata and long cells.ipynb" plotly_graphs.ipynb; do
        NB="$REAL_NB_DIR/$nb_name"
        [[ -f "$NB" ]] || continue
        BYTES=$(wc -c < "$NB" | tr -d ' ')

        run_bench "real notebook: ${nb_name}  [${BYTES} bytes]" \
            --command-name "rust" \
            "'$RUST_BIN' --from ipynb --to py:percent '$NB' --output '$TMP_DIR/real_out.py'" \
            --command-name "python" \
            "'$PY_BIN' --from ipynb --to py:percent '$NB' --output '$TMP_DIR/real_out_py.py'"
    done
fi

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "  Benchmark complete!"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
if $EXPORT; then
    echo "  JSON results saved to: $RESULTS_DIR/"
fi
echo ""
echo "  For Rust-internal criterion benchmarks run:"
echo "    cargo bench"
echo ""
