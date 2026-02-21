from fastapi import FastAPI
from pydantic import BaseModel
from sentence_transformers import SentenceTransformer
from typing import List

app = FastAPI()

model = SentenceTransformer('sentence-transformers/all-MiniLM-L6-v2')

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