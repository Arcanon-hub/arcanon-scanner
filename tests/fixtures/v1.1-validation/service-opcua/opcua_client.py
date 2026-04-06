# OPC-UA client service — intentionally does NOT import asyncua
# This file must produce ZERO py-opcua connections because import gate blocks it.
# The call shape "Client(" appears but without the asyncua import.
import logging

logger = logging.getLogger(__name__)


class GenericClient:
    """A generic client wrapper that mentions Client( in a docstring.

    Example:
        c = Client("opc.tcp://localhost:4840")
    """

    def __init__(self, url: str):
        self.url = url

    def connect(self):
        logger.info(f"Connecting to {self.url}")
