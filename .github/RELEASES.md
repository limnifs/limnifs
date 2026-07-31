# Releases

LimniFS releases are tag-driven. Push a `v*` tag to `main` and the
[release workflow](../.github/workflows/release.yml) runs end-to-end.

## Pipeline

1. **License scan** (`cargo-deny`): hard-fails on any GPL/AGPL/LGPL
   license anywhere in the dependency tree. Configuration lives in
   [`deny.toml`](../deny.toml). The license scan also runs on every
   PR via [ci.yml](../.github/workflows/ci.yml) so the boundary is
   enforced continuously, not only at release time.
2. **SBOM** (`cargo-cyclonedx`): one CycloneDX JSON per workspace
   crate, uploaded as a build artifact and attached to the GitHub
   Release.
3. **Reproducible build** (matrix):
   - `x86_64-unknown-linux-gnu`
   - `aarch64-unknown-linux-gnu`
   - `x86_64-apple-darwin`
   - `aarch64-apple-darwin`
4. **Checksums**: SHA256 per archive, published alongside the binary.
5. **GitHub Release**: archive + checksum + SBOM attached; release
   notes generated from commits since the last tag.

## Signing

v1 ships Ed25519 keypair signatures via `limni sign`. Signer holds a
private key; verifiers check with the corresponding public key.
Sigstore keyless (Fulcio + Rekor) is deferred — see
[TODO.impl/05-crypto/05-signing-sigstore.md](../TODO.impl/05-crypto/05-signing-sigstore.md).

## Cut a release

```sh
git checkout main
git pull
git tag -a v0.1.0 -m "v0.1.0: initial release"
git push origin v0.1.0
```

The workflow takes ~10 minutes. The GitHub Release page shows up at
<https://github.com/limnifs/limnifs/releases>.
