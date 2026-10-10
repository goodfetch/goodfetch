### Handler exception as seen by the engine

- **napi / throw**: `GenericFailure, boom: selector .price not found` (1 lines, stack frames: no)
- **napi / throwsync**: `PendingException, sync boom` (1 lines, stack frames: no)
- **rpc-node / throw**: `boom: selector .price not found` (6 lines, stack frames: yes)
- **rpc-node / throwsync**: `sync boom` (5 lines, stack frames: yes)
- **pyo3 / throw**: `Traceback (most recent call last):` (6 lines, stack frames: yes)
- **pyo3 / throwsync**: `ValueError: sync boom` (1 lines, stack frames: no)
- **rpc-py / throw**: `boom: selector .price not found` (10 lines, stack frames: yes)
- **rpc-py / throwsync**: `sync boom` (8 lines, stack frames: yes)
- **rpc-node / version skew**: {'runner': 'rpc-node', 'scen': 'skew', 'error': 'protocol version mismatch: client 2.0, engine 1.0', 'code': -32002}
- **rpc-py / version skew**: {'runner': 'rpc-py', 'scen': 'skew', 'error': 'protocol version mismatch: client 2.0, engine 1.0', 'code': -32002}
