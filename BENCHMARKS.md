# Performance Benchmarks

**Date**: 2025-12-27
**Project**: sol-parser-sdk
**Purpose**: Measure the impact of zero-latency optimizations

---

## Running Benchmarks

### Quick Start

```bash
# Run all benchmarks
cargo bench --bench zero_latency_optimizations

# Run specific benchmark group
cargo bench --bench zero_latency_optimizations -- "SmallVec Stack Allocation"
cargo bench --bench zero_latency_optimizations -- "Zero-Copy String"
cargo bench --bench zero_latency_optimizations -- "Discriminator Lookup"

# Generate HTML reports
cargo bench --bench zero_latency_optimizations
# Reports generated in: target/criterion/
```

### Prerequisites

- Release mode is required (benchmarks automatically use `--release`)
- At least 2-3 minutes for full benchmark suite
- Results saved to `target/criterion/` directory

---

## Benchmark Groups

### 1. SmallVec Stack Allocation

**Purpose**: Measure the performance benefit of stack allocation vs heap allocation for small vectors.

**Tests**:
- SmallVec with 1, 2, 3, 4 elements (stack allocated)
- SmallVec with 5, 8, 12 elements (heap allocated after spillover)
- Vec with same sizes (always heap allocated)

**Expected Results**:
- ✅ SmallVec faster for ≤4 elements (50-100ns savings per transaction)
- ✅ SmallVec same performance as Vec for >4 elements (no penalty)

**Interpretation**:
- Lower latency for SmallVec (1-4 elements) = optimization working
- Similar latency for both (5+ elements) = graceful heap spillover

---

### 2. Zero-Copy String Parsing

**Purpose**: Measure the performance benefit of returning `&str` references vs allocating `String`.

**Tests**:
- `read_string_ref()` vs `read_string()` for 3-byte, 20-byte, and 64-byte strings
- Zero-copy: Returns `&str` reference (no allocation)
- Allocation: Allocates `String` on heap

**Expected Results**:
- ✅ `read_string_ref` faster by 50-100ns for all string sizes
- ✅ Larger strings show bigger absolute time savings (more memcpy overhead)

**Interpretation**:
- Consistent 50-100ns gap = heap allocation overhead eliminated
- Gap increases with string length = memcpy cost scales with size

---

### 3. Text Field Extraction

**Purpose**: Measure zero-copy text parsing from log strings.

**Tests**:
- `extract_text_field_ref()` vs `extract_text_field()`
- Extract 3 fields from a log string

**Expected Results**:
- ✅ `extract_text_field_ref` faster by 150-300ns (3 fields × 50-100ns each)

**Interpretation**:
- Savings scale linearly with number of fields extracted
- Real transactions may have 2-5 text fields per event

---

### 4. Discriminator Lookup

**Purpose**: Compare hot-path sequential checks vs match statement.

**Tests**:
- Hot-path discriminator (first check) - PumpFun Trade
- Hot-path discriminator (second check) - Raydium CLMM
- Cold-path discriminator (match statement fallback)

**Expected Results**:
- ✅ Hot-path first check: ~1-2ns (immediate hit)
- ✅ Hot-path second check: ~3-4ns (one miss + one hit)
- ✅ Cold-path match: ~5-15ns (match overhead)

**Interpretation**:
- Hot-path checks should be faster than match statement
- Most common discriminators benefit most (40% of events)

---

### 5. Branch Prediction Hints

**Purpose**: Measure the effect of `likely()` hints on branch prediction.

**Tests**:
- Without `likely()` hint (normal if statement)
- With `likely()` hint (using `#[cold]` on unlikely branch)

**Test Data**: 90% true conditions (simulating hot-path frequency)

**Expected Results**:
- ✅ Marginal improvement (1-5ns) with `likely()` hint
- ✅ Benefit increases with higher prediction accuracy

**Interpretation**:
- Small individual benefit, but compounds across thousands of branches
- Real benefit comes from better CPU speculation

---

### 6. Realistic Event Parsing Scenarios

**Purpose**: End-to-end measurement combining multiple optimizations.

**Tests**:
1. **Small transaction** (2 events, no strings) - Best case for SmallVec
2. **Medium transaction** (4 events, no strings) - Still on stack
3. **Large transaction** (8 events, no strings) - Heap spillover
4. **Event with zero-copy string** - Combined optimization
5. **Event with allocated string** - Baseline comparison

**Expected Results**:
- ✅ Small/medium transactions: 50-100ns faster (SmallVec stack)
- ✅ With zero-copy string: Additional 50-100ns savings
- ✅ Large transactions: Similar performance (heap used in both)

**Interpretation**:
- Real-world transactions benefit from multiple optimizations
- 85% of transactions have ≤4 events (stack allocated)
- 30% of events have strings (zero-copy applies)

---

## Performance Targets

Based on our optimizations, expected improvements:

| Scenario | Before | After | Savings | % Improvement |
|----------|--------|-------|---------|---------------|
| 1 event, no strings | 150ns | 95ns | 55ns | 37% |
| 2 events, no strings | 200ns | 130ns | 70ns | 35% |
| 4 events, no strings | 250ns | 165ns | 85ns | 34% |
| With 3 string fields | 300ns | 130ns | 170ns | 57% |
| 8+ events | 350ns | 280ns | 70ns | 20% |

---

## Interpreting Results

### Reading Criterion Output

```
SmallVec Stack Allocation/SmallVec/2
                        time:   [12.345 ns 12.456 ns 12.567 ns]
                        ^^^^    ^^^^^^^^^           ^^^^^^^^^
                        metric  lower bound         upper bound
                                (95% confidence)
```

- **Lower bound**: Fastest measured time
- **Median**: Typical performance
- **Upper bound**: Slowest measured time (within 95% confidence)

### Performance Comparison

Criterion automatically compares to previous benchmark runs:

```
SmallVec Stack Allocation/SmallVec/2
                        time:   [12.345 ns 12.456 ns 12.567 ns]
                        change: [-15.23% -12.45% -9.67%] (p = 0.00 < 0.05)
                        Performance has improved.
```

- **Negative change %**: Performance improved (faster)
- **Positive change %**: Performance regressed (slower)
- **p-value < 0.05**: Statistically significant change

---

## Baseline vs Optimized

### Establishing Baseline

To measure the actual impact of optimizations:

1. **First run** (with all optimizations):
   ```bash
   cargo bench --bench zero_latency_optimizations
   ```

2. **Save baseline**:
   ```bash
   cp -r target/criterion target/criterion-baseline
   ```

3. **Compare after changes**:
   ```bash
   cargo bench --bench zero_latency_optimizations
   # Criterion will automatically show comparison
   ```

---

## Troubleshooting

### Unstable Results

If benchmark results vary significantly between runs:

1. **Close other applications** (reduce system noise)
2. **Disable CPU frequency scaling**:
   ```bash
   # Linux
   sudo cpupower frequency-set --governor performance

   # macOS
   # Use Activity Monitor to check CPU usage
   ```
3. **Run longer**:
   ```bash
   cargo bench --bench zero_latency_optimizations -- --measurement-time 30
   ```

### Out of Memory

If benchmarks crash due to memory:

1. **Reduce sample size** (edit benchmark configuration in code)
2. **Run specific benchmark groups** (not all at once)

---

## Benchmark Maintenance

### When to Re-run

- ✅ After implementing new optimizations
- ✅ Before major releases
- ✅ When upgrading Rust compiler (optimization improvements)
- ✅ When changing critical dependencies (e.g., smallvec version)

### What to Watch

- **Regressions**: Any benchmark slower than previous run
- **Inconsistency**: Wide confidence intervals (indicates unstable results)
- **Unexpected patterns**: E.g., SmallVec slower than Vec for small sizes

---

## Real-World Validation

Benchmarks are synthetic. For real-world validation:

1. **Production metrics**: Measure actual transaction parsing latency
2. **Load testing**: Use `examples/` with real blockchain data
3. **Profiling**: Use `perf` or `flamegraph` to find actual bottlenecks

---

## Benchmark Hygiene

### Best Practices

- ✅ Run on same hardware for consistency
- ✅ Run multiple times to establish confidence
- ✅ Document system specs (CPU, RAM, OS) when sharing results
- ✅ Use `black_box()` to prevent compiler optimizations
- ✅ Keep benchmarks simple and focused

### Anti-Patterns

- ❌ Optimizing for benchmarks (instead of real use cases)
- ❌ Cherry-picking favorable results
- ❌ Ignoring confidence intervals
- ❌ Benchmarking in debug mode

---

## Example: Running and Interpreting

```bash
$ cargo bench --bench zero_latency_optimizations -- "SmallVec Stack Allocation"

SmallVec Stack Allocation/SmallVec/1
                        time:   [8.234 ns 8.345 ns 8.456 ns]
SmallVec Stack Allocation/Vec/1
                        time:   [58.123 ns 58.456 ns 58.789 ns]

# Interpretation:
# - SmallVec (1 element): ~8ns   ✅ Stack allocation
# - Vec (1 element):      ~58ns  ❌ Heap allocation
# - Savings:              ~50ns  ✅ Matches expected 50-100ns

SmallVec Stack Allocation/SmallVec/8
                        time:   [62.123 ns 62.456 ns 62.789 ns]
SmallVec Stack Allocation/Vec/8
                        time:   [61.234 ns 61.567 ns 61.900 ns]

# Interpretation:
# - SmallVec (8 elements): ~62ns  ⚠️ Heap spillover (>4 elements)
# - Vec (8 elements):      ~61ns  ✅ Normal heap allocation
# - Difference:            ~1ns   ✅ No penalty for spillover
```

---

## Contributing

When adding new benchmarks:

1. Add to `benches/zero_latency_optimizations.rs`
2. Follow existing naming convention
3. Document what the benchmark measures
4. Update this README with interpretation guide

---

**Generated**: 2025-12-27
**Benchmark Suite**: `zero_latency_optimizations`
**Total Groups**: 6
**Total Benchmarks**: ~30 individual tests
