#!/usr/bin/env python3
"""
EXTREME Cache Performance Test Suite (Production-Grade)
========================================================

Rate-limit aware for Groq free tier (30 req/min).

Test Categories:
1. Realistic Debugging (10) - Actual error messages and debugging scenarios
2. Architecture & Implementation (10) - Real design and implementation questions  
3. Semantic Variations (12) - Complex queries with paraphrased variants
4. Temperature Variations (4) - Creativity settings
5. Max Tokens (3) - Response length limits
6. Concurrent Load (20) - Diverse queries under parallel load
7. Mixed Workload Stress (25) - Realistic usage patterns

Total: ~84 queries with production-grade complexity

Usage:
    # Run everything (default):
    python test_extreme_cache.py
    
    # Adjust rate limit for paid tier:
    python test_extreme_cache.py --rate-limit 60
    
    # Skip expensive tests:
    python test_extreme_cache.py --skip-concurrent --skip-stress
"""

import requests
import time
import json
import csv
import statistics
import threading
import random
import argparse
from datetime import datetime
from typing import List, Dict, Optional
from dataclasses import dataclass, asdict
from openai import OpenAI
from concurrent.futures import ThreadPoolExecutor, as_completed
from collections import deque

PROXY_URL = "http://localhost:3000"

client = OpenAI(
    base_url=f"{PROXY_URL}/v1",
    api_key="dummy-key",
)

@dataclass
class TestResult:
    scenario: str
    query: str
    cache_status: str
    latency_ms: float
    tokens: int
    cost_usd: float
    timestamp: str
    query_length: int = 0
    response_length: int = 0
    thread_id: Optional[str] = None
    error: Optional[str] = None

class RateLimiter:
    """Smart rate limiter that tracks requests and enforces limits"""
    def __init__(self, requests_per_minute: int = 30):
        self.rpm = requests_per_minute
        self.request_times = deque()
        self.lock = threading.Lock()
        self.wait_count = 0
        
    def wait_if_needed(self):
        """Wait if we're approaching rate limit"""
        with self.lock:
            now = time.time()
            
            # Remove requests older than 1 minute
            while self.request_times and self.request_times[0] < now - 60:
                self.request_times.popleft()
            
            # If we're at the limit, wait
            if len(self.request_times) >= self.rpm:
                wait_until = self.request_times[0] + 60
                wait_time = wait_until - now
                if wait_time > 0:
                    self.wait_count += 1
                    print(f"    ⏳ Rate limit: waiting {wait_time:.1f}s")
                    time.sleep(wait_time + 0.5)  # Add buffer
                    # Clean up old times after waiting
                    now = time.time()
                    while self.request_times and self.request_times[0] < now - 60:
                        self.request_times.popleft()
            
            # Record this request
            self.request_times.append(now)

class ExtremeCacheTester:
    def __init__(self, rate_limit_rpm: int = 30):
        self.results: List[TestResult] = []
        self.model = "llama-3.3-70b-versatile"
        self.error_count = 0
        self.lock = threading.Lock()
        self.rate_limiter = RateLimiter(rate_limit_rpm)
        self.rate_limit_rpm = rate_limit_rpm
        
    def clear_cache(self):
        try:
            response = requests.post(f"{PROXY_URL}/admin/cache/clear", timeout=5)
            if response.status_code == 200:
                print("✅ Cache cleared")
                time.sleep(1)
                return True
            print(f"⚠️  Failed to clear cache: {response.status_code}")
            return False
        except Exception as e:
            print(f"⚠️  Could not clear cache: {e}")
            return False
    
    def check_health(self) -> bool:
        try:
            response = requests.get(f"{PROXY_URL}/health", timeout=5)
            data = response.json()
            all_up = all(s["status"] == "up" for s in data.get("services", {}).values())
            if not all_up:
                print("⚠️  Warning: Some services are down:")
                for name, status in data.get("services", {}).items():
                    if status["status"] != "up":
                        print(f"    {name}: {status['status']}")
            return all_up
        except Exception as e:
            print(f"❌ Health check failed: {e}")
            return False
    
    def ask(self, query: str, scenario: str = "default", temperature: float = 0.0, 
            max_tokens: Optional[int] = None, thread_id: Optional[str] = None,
            skip_rate_limit: bool = False) -> TestResult:
        """Send query with automatic rate limiting"""
        
        # Wait if we're hitting rate limits
        if not skip_rate_limit:
            self.rate_limiter.wait_if_needed()
        
        start_time = time.time()
        try:
            response = client.chat.completions.create(
                model=self.model,
                messages=[{"role": "user", "content": query}],
                temperature=temperature,
                max_tokens=max_tokens,
            )
            
            latency_ms = (time.time() - start_time) * 1000
            tokens = response.usage.total_tokens
            response_text = response.choices[0].message.content
            cost_usd = tokens * 0.00000069
            
            if latency_ms < 5:
                cache_status = "EXACT_HIT"
            elif latency_ms < 100:
                cache_status = "SEMANTIC_HIT"
            else:
                cache_status = "MISS"
            
            result = TestResult(
                scenario=scenario,
                query=query[:100] + "..." if len(query) > 100 else query,
                cache_status=cache_status,
                latency_ms=round(latency_ms, 2),
                tokens=tokens,
                cost_usd=round(cost_usd, 6),
                timestamp=datetime.now().isoformat(),
                query_length=len(query),
                response_length=len(response_text),
                thread_id=thread_id,
            )
            
            with self.lock:
                self.results.append(result)
            return result
            
        except Exception as e:
            error_str = str(e)
            
            # Check if it's a rate limit error
            if "429" in error_str or "rate" in error_str.lower():
                print(f"    ⚠️  Rate limit hit! Waiting 60s...")
                time.sleep(60)
                return self.ask(query, scenario, temperature, max_tokens, thread_id, skip_rate_limit=True)
            
            latency_ms = (time.time() - start_time) * 1000
            result = TestResult(
                scenario=scenario,
                query=query[:100] + "..." if len(query) > 100 else query,
                cache_status="ERROR",
                latency_ms=round(latency_ms, 2),
                tokens=0,
                cost_usd=0.0,
                timestamp=datetime.now().isoformat(),
                query_length=len(query),
                response_length=0,
                thread_id=thread_id,
                error=error_str[:200]
            )
            with self.lock:
                self.results.append(result)
                self.error_count += 1
            return result
    
    def test_realistic_debugging(self):
        print("\n" + "="*70)
        print("🔥 TEST 1: Realistic Debugging Queries (10 queries)")
        print("="*70)
        
        queries = [
            ("I'm getting 'cannot borrow as mutable' error when trying to modify a vector inside a loop in Rust. How do I fix this?", "borrow_error_1"),
            ("Getting 'cannot borrow as mutable' in Rust loop with vector modification", "borrow_error_2"),
            ("My Rust code panics with 'index out of bounds'. How do I safely access vector elements?", "panic_bounds"),
            ("Why does my async Rust function not run? I'm using tokio but nothing happens.", "async_not_run"),
            ("How do I fix 'trait bound not satisfied' error when trying to use HashMap with custom struct?", "trait_bound"),
            ("Rust compiler says 'lifetime may not live long enough' in my function. What does this mean?", "lifetime_error"),
            ("I get 'move occurs because value has type' error. How do I fix ownership issues in Rust?", "move_error"),
            ("My Actix-web server returns 500 error but I don't see any logs. How to debug?", "actix_debug"),
            ("Why is my Rust program so slow compared to Python? Am I doing something wrong?", "perf_issue"),
            ("How do I properly handle errors in Rust without using unwrap everywhere?", "error_handling"),
        ]
        
        for query, label in queries:
            print(f"  {label:20s}: ", end="", flush=True)
            result = self.ask(query, scenario="realistic_debugging")
            print(f"{result.cache_status:15s} ({result.latency_ms:6.1f}ms)")
    
    def test_architecture_implementation(self):
        print("\n" + "="*70)
        print("🔥 TEST 2: Architecture & Implementation (10 queries)")
        print("="*70)
        
        queries = [
            ("How do I implement a thread-safe producer-consumer queue in Rust using channels and tokio?", "producer_consumer_1"),
            ("What's the best way to implement producer-consumer pattern in Rust with async channels?", "producer_consumer_2"),
            ("How to structure a REST API in Rust with Actix-web? Should I use one router or multiple modules?", "rest_structure"),
            ("What's the difference between Arc<Mutex<T>> and Arc<RwLock<T>>? When should I use each?", "arc_mutex_rwlock"),
            ("How do I implement JWT authentication in a Rust web API? Which crate should I use?", "jwt_auth"),
            ("What's the best way to handle database connection pooling in Rust with SQLx and async?", "db_pooling"),
            ("How to implement a custom iterator in Rust that filters and maps values lazily?", "custom_iterator"),
            ("Should I use Box, Rc, or Arc for my data structure in Rust? What are the tradeoffs?", "box_rc_arc"),
            ("How do I properly structure error handling in a large Rust project with custom error types?", "error_design"),
            ("What's the recommended way to organize a Rust workspace with multiple crates and shared dependencies?", "workspace_org"),
        ]
        
        for query, label in queries:
            print(f"  {label:20s}: ", end="", flush=True)
            result = self.ask(query, scenario="architecture")
            print(f"{result.cache_status:15s} ({result.latency_ms:6.1f}ms)")
    
    def test_semantic_boundary(self):
        print("\n" + "="*70)
        print("🔥 TEST 3: Semantic Variations (12 queries)")
        print("="*70)
        print("Testing if cache catches different phrasings of same question")
        
        # Group 1: OAuth implementation
        print(f"\n  Group 1: OAuth Implementation")
        oauth_queries = [
            ("How do I implement OAuth2 authentication in Rust with Actix-web?", "oauth_base"),
            ("What's the best way to add OAuth2 to an Actix-web application in Rust?", "oauth_var1"),
            ("How to handle OAuth2 authentication in Rust using Actix-web framework?", "oauth_var2"),
        ]
        for query, label in oauth_queries:
            print(f"    {label:15s}: ", end="", flush=True)
            result = self.ask(query, scenario="semantic_oauth")
            print(f"{result.cache_status:15s} ({result.latency_ms:6.1f}ms)")
        
        # Group 2: Error handling
        print(f"\n  Group 2: Error Handling")
        error_queries = [
            ("How do I properly handle errors in async Rust without using unwrap?", "error_base"),
            ("What's the best way to handle errors in Rust async code without unwrap?", "error_var1"),
            ("How to avoid unwrap and handle errors properly in Rust async functions?", "error_var2"),
        ]
        for query, label in error_queries:
            print(f"    {label:15s}: ", end="", flush=True)
            result = self.ask(query, scenario="semantic_error")
            print(f"{result.cache_status:15s} ({result.latency_ms:6.1f}ms)")
        
        # Group 3: Unrelated (should NOT match)
        print(f"\n  Group 3: Unrelated Queries (should miss)")
        unrelated = [
            ("How do I deploy a Rust application to AWS Lambda?", "unrelated_1"),
            ("What's the difference between async-std and tokio in Rust?", "unrelated_2"),
            ("How to write unit tests in Rust with mock objects?", "unrelated_3"),
        ]
        for query, label in unrelated:
            print(f"    {label:15s}: ", end="", flush=True)
            result = self.ask(query, scenario="semantic_unrelated")
            print(f"{result.cache_status:15s} ({result.latency_ms:6.1f}ms)")
    
    def test_concurrent(self, num_workers: int = 5, requests_per_worker: int = 4):
        print("\n" + "="*70)
        print(f"🔥 TEST 4: Concurrent Load ({num_workers}x{requests_per_worker}=20 queries)")
        print("="*70)
        print("Testing thread safety with diverse queries")
        
        # Real diverse queries that might hit concurrently
        queries = [
            "How do I handle database transactions in Rust with SQLx?",
            "What's the best way to implement rate limiting in Actix-web?",
            "How to handle file uploads in Rust web applications?",
            "How do I implement WebSocket connections in Actix-web?",
            "What's the difference between .await and .block_on() in Rust?",
            "How to handle CORS properly in a Rust API?",
            "How do I implement pagination for database queries in Rust?",
            "What's the best way to cache responses in an Actix-web application?",
        ]
        
        def worker(worker_id: int):
            for i in range(requests_per_worker):
                self.ask(random.choice(queries), scenario="concurrent", thread_id=f"w{worker_id}")
        
        start_time = time.time()
        with ThreadPoolExecutor(max_workers=num_workers) as executor:
            list(executor.map(worker, range(num_workers)))
        
        elapsed = time.time() - start_time
        total = num_workers * requests_per_worker
        print(f"  ✅ {total} requests in {elapsed:.1f}s ({total/elapsed:.1f} req/s)")
    
    def test_rapid_fire(self, num_requests: int = 25):
        print("\n" + "="*70)
        print(f"🔥 TEST 5: Mixed Workload Stress Test ({num_requests} queries)")
        print("="*70)
        print("Simulating realistic usage with repeated + new queries")
        
        # Mix of common questions (that should hit) and unique questions (that should miss)
        queries = [
            # Common questions (asked multiple times - should cache well)
            "How do I handle errors in Rust without unwrap?",
            "How do I handle errors in Rust without unwrap?",  # Repeat
            "What's the best way to avoid unwrap in Rust error handling?",  # Semantic variant
            
            # Another common pattern
            "How to implement async functions in Rust with Tokio?",
            "How to implement async functions in Rust with Tokio?",  # Repeat
            
            # Mix in unique queries (should miss)
            "How do I parse JSON in Rust using serde?",
            "What's the difference between String and &str in Rust?",
            "How to read files asynchronously in Rust?",
            "How do I create custom macros in Rust?",
            "What's the best way to handle configuration in Rust apps?",
        ]
        
        start_time = time.time()
        for i in range(num_requests):
            query = queries[i % len(queries)]
            self.ask(query, scenario="rapid_fire")
            if (i + 1) % 5 == 0:
                print(f"  Progress: {i+1}/{num_requests}", end="\r", flush=True)
        
        elapsed = time.time() - start_time
        print(f"\n  ✅ {num_requests} requests in {elapsed:.1f}s ({num_requests/elapsed:.1f} req/s)")
    
    def test_temperature(self):
        print("\n" + "="*70)
        print("🔥 TEST 6: Temperature (4 queries)")
        print("="*70)
        
        query = "Write a story about a robot learning Rust"
        for temp in [0.0, 0.7, 1.0]:
            print(f"  Temp {temp:.1f}: ", end="", flush=True)
            result = self.ask(query, scenario="temperature", temperature=temp)
            print(f"{result.cache_status:15s} ({result.latency_ms:6.1f}ms)")
        
        print(f"  Repeat 0.0: ", end="", flush=True)
        result = self.ask(query, scenario="temperature", temperature=0.0)
        print(f"{result.cache_status:15s} ({result.latency_ms:6.1f}ms)")
    
    def test_max_tokens(self):
        print("\n" + "="*70)
        print("🔥 TEST 7: Max Tokens (3 queries)")
        print("="*70)
        
        query = "Explain Rust ownership"
        for max_tokens in [50, 200, None]:
            print(f"  Max {str(max_tokens):4s}: ", end="", flush=True)
            result = self.ask(query, scenario="max_tokens", max_tokens=max_tokens)
            print(f"{result.cache_status:15s} got {result.tokens} tokens")
    
    def calculate_statistics(self) -> Dict:
        if not self.results:
            return {}
        
        total = len(self.results)
        exact = sum(1 for r in self.results if r.cache_status == "EXACT_HIT")
        semantic = sum(1 for r in self.results if r.cache_status == "SEMANTIC_HIT")
        misses = sum(1 for r in self.results if r.cache_status == "MISS")
        errors = sum(1 for r in self.results if r.cache_status == "ERROR")
        
        hit_rate = ((exact + semantic) / total * 100) if total > 0 else 0
        
        total_cost_spent = sum(r.cost_usd for r in self.results if r.cache_status == "MISS")
        total_cost_possible = sum(r.cost_usd for r in self.results if r.cache_status != "ERROR")
        cost_saved = total_cost_possible - total_cost_spent
        savings_pct = (cost_saved / total_cost_possible * 100) if total_cost_possible > 0 else 0
        
        def calc_stats(values):
            if not values:
                return {"mean": 0, "median": 0, "p95": 0}
            return {
                "mean": statistics.mean(values),
                "median": statistics.median(values),
                "p95": statistics.quantiles(values, n=20)[18] if len(values) > 1 else values[0],
            }
        
        scenarios = {}
        for scenario in set(r.scenario for r in self.results):
            sr = [r for r in self.results if r.scenario == scenario]
            hits = sum(1 for r in sr if r.cache_status in ["EXACT_HIT", "SEMANTIC_HIT"])
            scenarios[scenario] = {
                "total": len(sr),
                "hits": hits,
                "hit_rate": (hits / len(sr) * 100) if sr else 0
            }
        
        return {
            "total_requests": total,
            "exact_hits": exact,
            "semantic_hits": semantic,
            "misses": misses,
            "errors": errors,
            "hit_rate": hit_rate,
            "cost_saved_usd": cost_saved,
            "cost_spent_usd": total_cost_spent,
            "savings_percent": savings_pct,
            "tokens_saved": sum(r.tokens for r in self.results if r.cache_status in ["EXACT_HIT", "SEMANTIC_HIT"]),
            "tokens_used": sum(r.tokens for r in self.results if r.cache_status == "MISS"),
            "latency_exact": calc_stats([r.latency_ms for r in self.results if r.cache_status == "EXACT_HIT"]),
            "latency_semantic": calc_stats([r.latency_ms for r in self.results if r.cache_status == "SEMANTIC_HIT"]),
            "latency_miss": calc_stats([r.latency_ms for r in self.results if r.cache_status == "MISS"]),
            "by_scenario": scenarios,
            "rate_limit_waits": self.rate_limiter.wait_count,
        }
    
    def print_report(self, stats: Dict):
        print("\n" + "="*70)
        print("📊 EXTREME TEST RESULTS")
        print("="*70)
        
        print(f"\n🎯 Cache Performance:")
        print(f"  Total:        {stats['total_requests']}")
        print(f"  Exact Hits:   {stats['exact_hits']} ({stats['exact_hits']/stats['total_requests']*100:.1f}%)")
        print(f"  Semantic:     {stats['semantic_hits']} ({stats['semantic_hits']/stats['total_requests']*100:.1f}%)")
        print(f"  Misses:       {stats['misses']}")
        print(f"  Errors:       {stats['errors']}")
        print(f"  Hit Rate:     {stats['hit_rate']:.1f}%")
        
        print(f"\n💰 Cost:")
        print(f"  Saved:        ${stats['cost_saved_usd']:.4f} ({stats['savings_percent']:.1f}%)")
        print(f"  Spent:        ${stats['cost_spent_usd']:.4f}")
        
        print(f"\n⚡ Latency (ms):")
        if stats['latency_exact']['mean'] > 0:
            l = stats['latency_exact']
            print(f"  Exact:        {l['mean']:.1f} avg, {l['median']:.1f} median, {l['p95']:.1f} p95")
        if stats['latency_semantic']['mean'] > 0:
            l = stats['latency_semantic']
            print(f"  Semantic:     {l['mean']:.1f} avg, {l['median']:.1f} median, {l['p95']:.1f} p95")
        if stats['latency_miss']['mean'] > 0:
            l = stats['latency_miss']
            print(f"  Miss:         {l['mean']:.1f} avg")
            if stats['latency_exact']['mean'] > 0:
                print(f"  Speedup:      {l['mean']/stats['latency_exact']['mean']:.0f}x (exact vs miss)")
        
        print(f"\n📈 By Scenario:")
        for s, d in sorted(stats['by_scenario'].items()):
            print(f"  {s:20s}: {d['hits']}/{d['total']} ({d['hit_rate']:.0f}%)")
        
        print(f"\n⏳ Rate Limiting: {stats['rate_limit_waits']} waits")
        print("="*70)
    
    def export_results(self):
        ts = datetime.now().strftime("%Y%m%d_%H%M%S")
        
        csv_file = f"extreme_test_{ts}.csv"
        with open(csv_file, 'w', newline='') as f:
            writer = csv.DictWriter(f, fieldnames=asdict(self.results[0]).keys())
            writer.writeheader()
            for r in self.results:
                writer.writerow(asdict(r))
        
        json_file = f"extreme_summary_{ts}.json"
        with open(json_file, 'w') as f:
            json.dump(self.calculate_statistics(), f, indent=2)
        
        print(f"\n📄 Exported: {csv_file}, {json_file}")

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--rate-limit', type=int, default=30)
    parser.add_argument('--skip-concurrent', action='store_true')
    parser.add_argument('--skip-stress', action='store_true')
    args = parser.parse_args()
    
    print("🚀 EXTREME Cache Testing - PRODUCTION-GRADE QUERIES")
    print("="*70)
    print(f"⚙️  {args.rate_limit} req/min limit")
    print(f"⏱️  ~84 total requests with REALISTIC complexity")
    print(f"📝  Real debugging, architecture, and implementation questions")
    print(f"🎯  Designed to test actual production scenarios")
    print("="*70)
    
    tester = ExtremeCacheTester(args.rate_limit)
    
    if not tester.check_health():
        if input("\n⚠️  Services down. Continue? (y/n): ").lower() != 'y':
            return
    
    tester.clear_cache()
    
    # Run ALL tests by default
    print("\n🔥 Running ALL extreme tests with REALISTIC queries...")
    print("   (Tests will auto-throttle to respect rate limits)\n")
    
    tester.test_realistic_debugging()
    tester.test_architecture_implementation()
    tester.test_semantic_boundary()
    tester.test_temperature()
    tester.test_max_tokens()
    
    if not args.skip_concurrent:
        tester.test_concurrent()
    if not args.skip_stress:
        tester.test_rapid_fire()
    
    stats = tester.calculate_statistics()
    tester.print_report(stats)
    tester.export_results()
    
    print(f"\n✅ Complete! Errors: {tester.error_count}, Rate waits: {tester.rate_limiter.wait_count}")

if __name__ == "__main__":
    main()