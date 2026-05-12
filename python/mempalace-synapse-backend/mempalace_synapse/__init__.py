"""mempalace-synapse-backend — registers 'synapse' as a MemPalace backend."""

from .backend import SynapseBackend, SynapseCollection, SynapseRpcBackend, SynapseRpcCollection

__all__ = ["SynapseBackend", "SynapseCollection", "SynapseRpcBackend", "SynapseRpcCollection"]
