#!/bin/bash

echo "🚀 Generating traffic for dashboard..."
echo ""

# Request 1: First query (MISS)
echo "1️⃣  Request 1: What is Rust? (MISS)"
curl -s -X POST http://localhost:3000/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model": "llama-3.3-70b-versatile", "messages": [{"role": "user", "content": "What is Rust?"}]}' > /dev/null
sleep 1

# Request 2: Same query (EXACT HIT)
echo "2️⃣  Request 2: What is Rust? (EXACT HIT)"
curl -s -X POST http://localhost:3000/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model": "llama-3.3-70b-versatile", "messages": [{"role": "user", "content": "What is Rust?"}]}' > /dev/null
sleep 1

# Request 3: Paraphrase (SEMANTIC HIT)
echo "3️⃣  Request 3: Tell me about Rust (SEMANTIC HIT)"
curl -s -X POST http://localhost:3000/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model": "llama-3.3-70b-versatile", "messages": [{"role": "user", "content": "Tell me about Rust"}]}' > /dev/null
sleep 1

# Request 4: New query (MISS)
echo "4️⃣  Request 4: What is Python? (MISS)"
curl -s -X POST http://localhost:3000/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model": "llama-3.3-70b-versatile", "messages": [{"role": "user", "content": "What is Python?"}]}' > /dev/null
sleep 1

# Request 5: Exact hit
echo "5️⃣  Request 5: What is Python? (EXACT HIT)"
curl -s -X POST http://localhost:3000/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model": "llama-3.3-70b-versatile", "messages": [{"role": "user", "content": "What is Python?"}]}' > /dev/null
sleep 1

# Request 6: Semantic hit
echo "6️⃣  Request 6: Explain Rust language (SEMANTIC HIT)"
curl -s -X POST http://localhost:3000/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model": "llama-3.3-70b-versatile", "messages": [{"role": "user", "content": "Explain Rust programming language"}]}' > /dev/null
sleep 1

# Request 7-10: More variety
echo "7️⃣  Request 7: New topic (MISS)"
curl -s -X POST http://localhost:3000/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model": "llama-3.3-70b-versatile", "messages": [{"role": "user", "content": "How does async work?"}]}' > /dev/null
sleep 1

echo "8️⃣  Request 8: Bypass cache test"
curl -s -X POST http://localhost:3000/v1/chat/completions \
  -H "Content-Type: application/json" \
  -H "X-Bypass-Cache: true" \
  -d '{"model": "llama-3.3-70b-versatile", "messages": [{"role": "user", "content": "What is Rust?"}]}' > /dev/null
sleep 1

echo "9️⃣  Request 9: Short TTL test"
curl -s -X POST http://localhost:3000/v1/chat/completions \
  -H "Content-Type: application/json" \
  -H "X-Cache-TTL: 10" \
  -d '{"model": "llama-3.3-70b-versatile", "messages": [{"role": "user", "content": "Quick cache test"}]}' > /dev/null
sleep 1

echo "🔟 Request 10: Another exact hit"
curl -s -X POST http://localhost:3000/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model": "llama-3.3-70b-versatile", "messages": [{"role": "user", "content": "How does async work?"}]}' > /dev/null

echo ""
echo "✅ Done! Check metrics:"
echo "   curl http://localhost:3000/metrics | jq"