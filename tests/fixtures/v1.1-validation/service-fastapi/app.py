from fastapi import FastAPI
from kubernetes import client as k8s_client
import asyncua

app = FastAPI()

DOCSTRING_DECOY = """
This docstring mentions various API calls like:
  - CoreV1Api() for pod management
  - Client("opc.tcp://example:4840") for OPC-UA
These must NOT be detected as connections (DACC-04).
"""


@app.get("/items")
async def list_items():
    """List all items.

    Also connects to CoreV1Api() inside this docstring -- must not fire.
    """
    return []


@app.get("/pods")
async def list_pods():
    # This REAL call must produce a kubernetes connection (DACC-05)
    v1 = k8s_client.CoreV1Api()
    pods = v1.list_pod_for_all_namespaces()
    return [p.metadata.name for p in pods.items]


@app.get("/opcua-status")
async def opcua_status():
    # This REAL asyncua call must produce an opcua connection (DACC-01 positive case)
    c = asyncua.Client("opc.tcp://plc.example.com:4840")
    return {"url": "opc.tcp://plc.example.com:4840"}
