from fastapi import FastAPI
from pydantic import BaseModel
from sentence_transformers import SentenceTransformer
from typing import List

app = FastAPI()

print("Loading embedding model (all-MiniLM-L6-v2)...", flush=True)
print("This may take a minute on first run while the model downloads (~80MB).", flush=True)
model = SentenceTransformer('sentence-transformers/all-MiniLM-L6-v2')
print("Embedding model loaded successfully. Ready to serve requests.", flush=True)

class EmbeddingRequest(BaseModel):
    text: str

class EmbeddingResponse(BaseModel):
    embedding: List[float]

@app.post("/embed")
async def get_embedding(request: EmbeddingRequest) -> EmbeddingResponse:
    embedding = model.encode(request.text)
    return EmbeddingResponse(embedding = embedding.tolist())

@app.get("/health")
async def health():
    return { "status": "ok"}