from fastapi import FastAPI
import httpx

app = FastAPI()


@app.get("/items")
async def list_items():
    return []


@app.post("/items")
async def create_item():
    async with httpx.AsyncClient() as client:
        resp = await client.get("http://service-a/users")
    return {}
