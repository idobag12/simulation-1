# ADR 0002 — Phase 0 implementation choices and dependency set

- Status: Accepted
- Date: 2026-07-09
- Phase: 0

The spec underdetermines several Phase 0 mechanics. Per SPEC §16.7, each
choice is recorded here before the code that embodies it.

## 1. Canonical codec: bincode v2 (serde bridge), one config point

All persisted and hashed structures pass through `core_types::codec`, which
wraps `bincode::serde::{encode_to_vec, decode_from_slice}` with
`bincode::config::standard()`. Rationale: bincode v2 is the maintained line;
the serde bridge keeps us on plain `#[derive(Serialize, Deserialize)]`;
funneling every encode/decode through one module guarantees a single encoding
configuration exists in the codebase. `decode` rejects trailing bytes so a
truncated or over-long blob can never half-load silently.

## 2. Stable world hashing: in-house FNV-1a 64

`hash_world()` is FNV-1a (64-bit) over canonical encoded bytes, implemented
in ~30 lines in `core_types::hash` (`StableHasher`). Rationale: Rust's
`DefaultHasher` is not guaranteed stable across compiler releases and
`RandomState` is explicitly randomized; a dependency (xxhash, blake3) would
need its own ADR and buys nothing for this use. This hash detects state
divergence between runs of the *same* build and inputs — it is not
cryptographic and collision resistance is not a requirement (a divergent run
producing a colliding hash at every single 10k-tick checkpoint is not a
realistic failure mode). Every hashed field is length-framed
(name, length, bytes) so boundary shifts cannot alias. Golden-value tests
freeze the algorithm.

## 3. RNG stream seed derivation (frozen algorithm)

`derive_stream_seed(master: Seed, name: &str) -> [u8; 32]`:

1. `h = fnv1a64(name.as_bytes())`
2. `state = splitmix64_mix(master) ^ h`
3. Emit 4 successive `splitmix64` outputs from `state`, little-endian, as the
   32-byte PCG64 seed.

Rationale: `SeedableRng::seed_from_u64` is an implementation detail of
`rand_core` that may change between major versions; saves and replays must
never depend on it. This function is ours, documented, test-frozen with
golden values, and never changes (a change would be a save-format break
requiring `format_version` + migration).

`RngRegistry` stores streams in a `BTreeMap<String, Pcg64>` (deterministic
iteration/serialization order). Streams are created lazily on first use;
because the seed depends only on (master, name), a stream first touched
before or after a save/load round-trip has identical output.

## 4. RNG lives in `World`; `TickContext` stays immutable

SPEC §6 fixes `System::run(&mut self, world: &mut World, ctx: &TickContext)`.
Systems need `&mut` RNG access, so the `RngRegistry` is owned by `World`
(`world.rng("stream.name")`) and `TickContext` carries only tick data. This
keeps the spec's immutable-context signature while giving systems one
mutable handle (the world) to all simulation state — which is also exactly
the boundary that save/hash must capture.

## 5. Two signature extensions to `System::run` (spec deviation, declared)

`fn run(&mut self, world, ctx, cmd: &mut CommandBuffer) -> Result<(), EcsError>`

- **`cmd` parameter**: SPEC §6 mandates the command-buffer pattern with
  application "at a defined point after each system runs" — so the scheduler,
  not the system, must own application. The buffer therefore appears in the
  signature; the schedule runner applies it immediately after each system.
- **`Result` return**: SPEC §3 bans `unwrap`/`expect` in simulation code and
  requires typed propagating errors; a `()`-returning run method would force
  panics or swallowed errors. Errors abort the tick and propagate to the
  runner, wrapped with the failing system's name.

## 6. Component identity and store serialization

- Components implement `Component` with an explicit
  `const NAME: &'static str` (e.g. `"fixture.wealth"`) and a
  `const STORAGE: StorageKind`. `std::any::type_name` never appears in a save
  or hash (it is not stable across compiler versions).
- Both dense (`Vec<Option<T>>`) and sparse (`BTreeMap<u32, T>`) stores
  serialize to the same **canonical form**: the ascending
  `Vec<(u32, T)>` of live pairs. This makes hash and save independent of
  internal capacity, trailing-`None` padding, or a later change of a
  component's storage kind.
- Registration order is part of the save/hash format. Each application
  registers all components in one function, in a fixed documented order;
  loading is strict two-ways (a save blob without a registered store, or a
  registered store without a save blob, is an error, never a default).

## 7. Entity allocator

Generational indices; freed indices are recycled LIFO from an explicit
`Vec<u32>` stack (deterministic, serialized verbatim). Generations bump on
despawn using wrapping arithmetic — a u32 generation wrap requires 2^32
despawns of one slot and the resulting ABA window is accepted and documented.
Despawn removes the entity's components from every store at despawn time
(registration order), so stores never carry dead-entity data.

## 8. Save format v1

`[8-byte magic "EMBRSAV1"][u32 LE format_version][zstd(codec(SaveBody))]`
where `SaveBody = { seed, tick, entities, rng, components: Vec<(name, bytes)> }`.
The version header sits outside the compressed payload so migrations can
route before decoding. `migrations::migrate_to_current` is the permanent
seam: v1 is pass-through; unknown versions are a typed error. The
"intermediate dynamic representation" machinery arrives with the first real
migration (v2), per YAGNI — building it against a single version would be
speculative complexity.

zstd level is a named constant (compression level affects only file size,
never simulation behavior — it is not a balance tunable and does not belong
in `data/balance/`).

## 9. Phase 0 schedule has only the per-tick rate

SPEC §6 lists five rates (tick/hour/day/season/year), but hour/day/season/
year firing rules are defined by the `Calendar`, which is Phase 1 scope. The
`Schedule` therefore carries only the tick-rate system list in Phase 0; the
other rates are added with the Calendar in Phase 1. Clean seam, not a stub:
no placeholder rates exist.

## 10. Determinism fixture lives in `headless`, clearly labeled

The determinism suite needs state that actually evolves (components mutating,
entities spawning/despawning, RNG streams advancing). Phase 0 has no
simulation content by design, so `headless::fixture` provides
`FixtureWealth` (dense store), `FixtureTag` (sparse store), and
`FixtureWalkSystem` — a random-walk exerciser used by tests and behind the
explicit `--fixture` CLI flag. This is test tooling, not simulation content:
it is not a stub of any future feature, it is named as a fixture, and it is
never registered in a schedule unless explicitly requested. Its numeric
constants are fixture parameters, not balance tunables (SPEC §8's grep test
applies to sim crates; `headless` is tooling).

## 11. Hand-rolled CLI argument parsing

The `headless` CLI parses `std::env::args` directly (~100 lines) instead of
adding `clap`. Rationale: the dependency policy (SPEC §2) puts the burden on
new dependencies; Phase 0 needs three subcommands and a handful of flags.
Revisit via ADR when the CLI's surface grows enough to earn it.

## 12. Lint enforcement of the coding standards

Every crate sets `#![forbid(unsafe_code)]` and `#![warn(missing_docs)]`;
core/sim/persistence crates additionally
`#![warn(clippy::unwrap_used, clippy::expect_used, clippy::panic)]`.
`scripts/check.sh` runs clippy with `-D warnings`, turning all of these into
hard failures. `clippy.toml` sets `allow-unwrap-in-tests = true` /
`allow-expect-in-tests = true`, matching SPEC §3's test/tooling exemption.

## Dependencies (SPEC §2 requires written justification)

| Crate | Why | Sanctioned by |
|---|---|---|
| `serde` (derive) | save/hash serialization | SPEC §2 explicitly |
| `bincode` v2 (`serde` feature) | binary codec for saves/hashing | SPEC §2 explicitly |
| `rand_pcg` (`serde` feature) | the mandated `Pcg64` | SPEC §2 explicitly |
| `rand_core` | `RngCore`/`SeedableRng` traits `rand_pcg` is built on | implied by `rand_pcg` |
| `thiserror` | typed errors | SPEC §3 explicitly |
| `zstd` | save compression | SPEC §9 explicitly ("zstd-compressed") |

No other external dependencies exist in Phase 0. `proptest` and `criterion`
join in the phases that first need them (SPEC §14).
