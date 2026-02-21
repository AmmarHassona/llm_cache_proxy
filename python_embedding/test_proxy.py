#!/usr/bin/env python3
"""
Test the LLM Cache Proxy using the OpenAI Python SDK.
The proxy implements the OpenAI API format, so any app using
the OpenAI SDK can point at it with just a base_url change.

Usage:
    source venv/bin/activate
    python test_proxy.py
"""

import requests
from openai import OpenAI

PROXY_URL = "http://localhost:3000"

client = OpenAI(
    base_url=f"{PROXY_URL}/v1",
    api_key="dummy-key",  # proxy uses its own GROQ_API_KEY internally
)


def ask(question: str, label: str) -> str:
    print(f"=== {label} ===")
    print(f"Q: {question}")
    response = client.chat.completions.create(
        model="llama-3.3-70b-versatile",
        messages=[{"role": "user", "content": question}],
        temperature=0.0,
    )
    answer = response.choices[0].message.content
    print(f"A: {answer}\n")
    return answer


def print_metrics():
    print("=== Metrics ===")
    try:
        data = requests.get(f"{PROXY_URL}/metrics", timeout=5).json()
        perf = data["cache_performance"]
        print(f"  Total requests : {perf['total_requests']}")
        print(f"  Exact hits     : {perf['exact_hits']}")
        print(f"  Semantic hits  : {perf['semantic_hits']}")
        print(f"  Misses         : {perf['misses']}")
        print(f"  Hit rate       : {perf['hit_rate_percent']}")
        print(f"  Tokens saved   : {data['token_usage']['tokens_saved']}")
        print(f"  Cost saved     : {data['cost_analysis']['cost_saved_usd']}")
    except Exception as e:
        print(f"  Could not fetch metrics: {e}")


if __name__ == "__main__":
    # 1. Cache miss — fresh question, hits Groq API
    ask("What is the capital of France?", "Request 1: Cache MISS (fresh question)")

    # 2. Exact hit — identical question, served from Redis
    ask("What is the capital of France?", "Request 2: EXACT HIT (same question)")

    # 3. Semantic hit — rephrased question, served from Qdrant
    ask("Which city serves as France's capital?", "Request 3: SEMANTIC HIT (rephrased)")

    # 4. Another semantic hit with different phrasing
    ask("Tell me the capital city of France.", "Request 4: SEMANTIC HIT (different phrasing)")

    # 5. Unrelated question — should be a miss
    ask("What is the boiling point of water?", "Request 5: Cache MISS (new topic)")

    print_metrics()
