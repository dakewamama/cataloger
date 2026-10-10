# SBPF execution identity

## Finding

OpenZeppelin's May 27, 2026 analysis shows that legacy SBPF relocations can alter arbitrary in-bounds bytes before verification. A raw ELF hash therefore identifies stored bytes, not the relocated image. The demonstrated technique applies to SBPF v0, v1 and v2. SBPF v3 uses static syscalls and stricter ELF loading; Agave 3.1.10 selects that path from `e_flags` and does not apply the legacy relocation table.

The official Subscriptions devnet capture is ProgramData `HaYb5J9eXooZuNzN3z6TfuzVDcaTfiDdDPWCFtexFfMg`, observed finalized at slot `509022453`, for program `De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44`. Its exact padded ELF is 133280 bytes, SHA256 `2675ad1d2b5068d47fc5d169156cf4859a9c21c0406ce63e3828e3b7320fddbf`. ELF `e_flags` is `0`, which Agave 3.1.10 maps to SBPF v0. It is in the affected legacy loader path.

The relocation table contains 298 entries: 250 `R_BPF_64_RELATIVE` (type 8) and 48 `R_BPF_64_32` (type 10). Agave 3.1.10 also resolves in-range relative call instructions while loading, even when no dynamic relocation entry represents them. The loader then verifies the loaded bytecode. The pinned upstream `solana-sbpf` 0.13.1 loader successfully loaded and verified the capture. Under the recorded local profile, the loaded text hash is `sha256:1a1599901b2a60f30439f124e885d4b74d7b4162bb8600fe305d3c92705e4695`, loaded read-only image hash is `sha256:5264fe14b36e64a47d5c64d9207d57041334fd580dea7b7e0a5c570405ea0bf0`, and effective image fingerprint is `sha256:c7b657c1af45e65827213bb63da361081e5246440d56298bfb0c7e656f806cc2`.

The official build used `solana-verify 0.5.2`, Agave 3.1.10 and platform-tools v1.52. The full payload hash differs from `solana-verify`'s normalized hash because verification strips trailing zero padding. The normalized hash `e705f5a309f84f849b402f20de4bea5f2cc1d1d4f691ba7caabcb07c8b46af51` matches both the official build and our locked rebuild. These are distinct raw-storage and reproducible-build identities.

## What Cataloger records

Cataloger now retains the exact ProgramData address and evidence slot, source revision and build-toolchain claim when supplied, raw ELF SHA256, SBPF version, relocation counts by type, pinned SBPF loader revision, loader-configuration and syscall-registry hash, loaded text and read-only hashes, and a domain-separated fingerprint over the loaded image, entry point, addresses and function registry. The implementation uses upstream Agave and SBPF loader/parser/verifier APIs. It does not implement an ELF decoder or relocation engine.

An executable upgrade changes the raw identity. A later bank then fails the verified deployment hash and interval checks; the previous adapter record does not authorize interpreting the new bytes. The local inspection command is:

```sh
cargo run --offline -p catalyst-indexer --bin authorization-api -- inspect-executable /path/to/program.so
```

## Limits

The finalized capture has no authenticated validator build, active feature set or runtime-environment witness. The local profile uses Agave 3.1.10's environment factory and `SVMFeatureSet::all_enabled`; this deliberately does not stand in for devnet's exact bank environment. Cataloger requires a matching runtime witness before it accepts exact live support, so the captured Subscriptions record remains `Incomplete` for semantic interpretation.

The effective fingerprint is a deterministic fingerprint of Agave's loaded SBPF image under the named local profile. It is not JIT machine code, a cryptographic validator attestation, or proof that every validator ran that exact configuration. Runtime evidence is an operator trust input and must be independently sourced. Unknown flags, unsupported versions, malformed ELF, invalid relocations, changed ProgramData or missing runtime/source evidence fail closed.

This milestone does not assert exact live execution identity for Subscriptions and does not establish that all validators shared one runtime environment. No ARM semantics changed.
