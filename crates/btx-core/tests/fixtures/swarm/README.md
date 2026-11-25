# Real answers from a ModelNet engine

Captured 2026-11-25 from btxd at upstream 074dac16 built with the model
network (easynode CI run 45598950201, artifact 15015833815), two nodes on
regtest on loopback, started with the app's observing flags
(`btx_core::swarm::OBSERVING`) plus `-modelbind=127.0.0.1:<port>
-modellandiscovery=0` and, for `onepeer`, `-modelpeer=<the other node>`.

- `empty.*`: one node alone. No model, no peer.
- `onepeer.*`: two nodes pointed at each other; taken 5 seconds after start.
- `off.*`: the same engine started with `-modelnet=0`.

`swarm.rs` reads these in its tests. The home folder in paths was replaced.
