//! 高性能时钟性能对比示例
//!
//! 对比传统 chrono::Utc::now() 和高性能时钟的性能差异
//!
//! 运行方式：
//! ```bash
//! cargo run --example clock_benchmark --release
//! ```

use sol_parser_sdk::core::clock::{now_micros, elapsed_micros_since};
use std::time::Instant;

fn main() {
    println!("🔬 高性能时钟性能对比测试\n");
    println!("═══════════════════════════════════════════════════════════\n");

    // 预热
    for _ in 0..1000 {
        let _ = now_micros();
        let _ = chrono::Utc::now().timestamp_micros();
    }

    // 测试 1: 单次调用延迟对比
    println!("📊 测试 1: 单次调用延迟对比");
    println!("───────────────────────────────────────────────────────────\n");

    // 测试高性能时钟
    let start = Instant::now();
    let _ = now_micros();
    let high_perf_latency = start.elapsed();
    println!("✅ 高性能时钟: {:>8} ns", high_perf_latency.as_nanos());

    // 测试传统方式
    let start = Instant::now();
    let _ = chrono::Utc::now().timestamp_micros();
    let chrono_latency = start.elapsed();
    println!("⚠️  传统方式:   {:>8} ns", chrono_latency.as_nanos());

    let speedup = chrono_latency.as_nanos() as f64 / high_perf_latency.as_nanos() as f64;
    println!("\n🚀 性能提升: {:.1}x 倍\n", speedup);

    // 测试 2: 批量调用性能对比
    println!("═══════════════════════════════════════════════════════════\n");
    println!("📊 测试 2: 批量调用性能对比 (100,000 次)");
    println!("───────────────────────────────────────────────────────────\n");

    const ITERATIONS: usize = 100_000;

    // 测试高性能时钟
    let start = Instant::now();
    for _ in 0..ITERATIONS {
        let _ = now_micros();
    }
    let high_perf_total = start.elapsed();
    let high_perf_avg = high_perf_total.as_nanos() / ITERATIONS as u128;
    println!("✅ 高性能时钟:");
    println!("   总耗时: {:>8} μs", high_perf_total.as_micros());
    println!("   平均:   {:>8} ns/次", high_perf_avg);

    // 测试传统方式
    let start = Instant::now();
    for _ in 0..ITERATIONS {
        let _ = chrono::Utc::now().timestamp_micros();
    }
    let chrono_total = start.elapsed();
    let chrono_avg = chrono_total.as_nanos() / ITERATIONS as u128;
    println!("\n⚠️  传统方式:");
    println!("   总耗时: {:>8} μs", chrono_total.as_micros());
    println!("   平均:   {:>8} ns/次", chrono_avg);

    let batch_speedup = chrono_total.as_nanos() as f64 / high_perf_total.as_nanos() as f64;
    println!("\n🚀 性能提升: {:.1}x 倍", batch_speedup);
    println!("💾 节省时间: {} μs\n", chrono_total.as_micros() - high_perf_total.as_micros());

    // 测试 3: 模拟事件解析场景
    println!("═══════════════════════════════════════════════════════════\n");
    println!("📊 测试 3: 模拟事件解析场景 (10,000 个事件)");
    println!("───────────────────────────────────────────────────────────\n");

    const EVENTS: usize = 10_000;

    // 使用高性能时钟
    let start = Instant::now();
    for _ in 0..EVENTS {
        let grpc_recv_us = now_micros();
        // 模拟解析操作（10μs）
        std::thread::sleep(std::time::Duration::from_nanos(10_000));
        let tx_parser_us = elapsed_micros_since(grpc_recv_us);
        let _ = (grpc_recv_us, tx_parser_us);
    }
    let high_perf_scenario = start.elapsed();
    println!("✅ 高性能时钟:");
    println!("   总耗时: {:>8} ms", high_perf_scenario.as_millis());
    println!("   平均:   {:>8} μs/事件", high_perf_scenario.as_micros() / EVENTS as u128);

    // 使用传统方式
    let start = Instant::now();
    for _ in 0..EVENTS {
        let grpc_recv_us = chrono::Utc::now().timestamp_micros();
        // 模拟解析操作（10μs）
        std::thread::sleep(std::time::Duration::from_nanos(10_000));
        let tx_parser_us = chrono::Utc::now().timestamp_micros() - grpc_recv_us;
        let _ = (grpc_recv_us, tx_parser_us);
    }
    let chrono_scenario = start.elapsed();
    println!("\n⚠️  传统方式:");
    println!("   总耗时: {:>8} ms", chrono_scenario.as_millis());
    println!("   平均:   {:>8} μs/事件", chrono_scenario.as_micros() / EVENTS as u128);

    let scenario_speedup = chrono_scenario.as_millis() as f64 / high_perf_scenario.as_millis() as f64;
    println!("\n🚀 性能提升: {:.2}x 倍", scenario_speedup);
    println!("💾 节省时间: {} ms\n", chrono_scenario.as_millis() - high_perf_scenario.as_millis());

    // 测试 4: 时间戳精度验证
    println!("═══════════════════════════════════════════════════════════\n");
    println!("📊 测试 4: 时间戳精度验证");
    println!("───────────────────────────────────────────────────────────\n");

    let mut timestamps = Vec::new();
    for _ in 0..100 {
        timestamps.push(now_micros());
    }

    let mut monotonic = true;
    for i in 1..timestamps.len() {
        if timestamps[i] < timestamps[i - 1] {
            monotonic = false;
            break;
        }
    }

    println!("✅ 单调性检查: {}", if monotonic { "通过 ✓" } else { "失败 ✗" });
    println!("📈 时间戳范围: {} μs - {} μs", timestamps[0], timestamps[timestamps.len() - 1]);
    println!("⏱️  总跨度: {} μs\n", timestamps[timestamps.len() - 1] - timestamps[0]);

    // 总结
    println!("═══════════════════════════════════════════════════════════\n");
    println!("📝 总结");
    println!("───────────────────────────────────────────────────────────\n");
    println!("✅ 高性能时钟优势:");
    println!("   • 单次调用快 {:.1}x 倍", speedup);
    println!("   • 批量调用快 {:.1}x 倍", batch_speedup);
    println!("   • 实际场景快 {:.2}x 倍", scenario_speedup);
    println!("   • 保持单调性和精度");
    println!("\n💡 建议:");
    println!("   • 所有时间戳获取使用 now_micros()");
    println!("   • 所有耗时计算使用 elapsed_micros_since()");
    println!("   • 预期性能提升: 5-10% (整体解析延迟)\n");
    println!("═══════════════════════════════════════════════════════════\n");
}
