"""Synapse Memory Python SDK — unix socket client for synapsed daemon.

Example:
    >>> from synapse_memory import Client
    >>> c = Client()
    >>> c.put("trailbase chosen over pocketbase", title="decision/backend")
    >>> hits = c.search("backend choice?", mode="hybrid", limit=5)
"""
from .client import Client, SynapseError

__version__ = "0.1.0"
__all__ = ["Client", "SynapseError"]
