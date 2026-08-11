# Releasing

Transom updates itself, which changes what a release is. A published release is
not just a download page — it is an instruction that every installed copy will
act on. Two consequences run through everything below: the signing key matters
more than the binaries, and a release that is wrong in the wrong way cannot be
taken back, because installs will already have acted on it.

## The signing key

Updates are verified against a [minisign](https://jedisct1.github.io/minisign/)
public key compiled into the app. The private key exists to sign release
artifacts in CI and for nothing else.

| | |
| --- | --- |
| Key ID | `624CA433A2C57730` |
| Public key | `src-tauri/tauri.conf.json` → `plugins.updater.pubkey` (committed) |
| Private key | Password manager, entry *Transom — updater signing key* |
| Offline copy | *(record the second location here)* |
| CI secret | `TAURI_SIGNING_PRIVATE_KEY` |
| CI passphrase | `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` (currently empty) |

The key ID is recorded so a restored backup can be checked before it is trusted.
It is public information; it identifies the key, it does not grant use of it. It
is the value in the key file's own comment line:

```
untrusted comment: minisign public key: 624CA433A2C57730
```

Expect to meet it in two forms. minisign prints the ID byte-reversed, so the
same key reads as `3077C5A233A44C62` when the raw bytes are dumped — which is
what the verification snippet below compares. Different order, same key; a
restored backup showing either one is the right key.

**No developer machine needs the private key.** CI signs every release, and local
bundle builds use a throwaway key — see CONTRIBUTING.md. If a copy is sitting in
`~/.tauri/` on a laptop, back it up and then delete it.

### Losing it is worse than leaking it

This is the opposite of the usual instinct, and it should drive how the key is
handled.

If the key **leaks**, it is recoverable: you still have it, so you can ship one
more update — signed with the old key — that carries a new public key, and
installs roll forward on their own. Someone holding a stolen key also needs
write access to this repository's releases before they can deliver anything to
anyone.

If the key is **lost**, nothing can be done remotely. No future release will be
accepted by any existing install, every user is frozen on the version they have,
and each one has to find and reinstall the app by hand. Most never will.

So favour redundancy over secrecy: at least two independent backups, one of them
offline, and confirm a restore actually works before relying on it.

## Cutting a release

The version appears in three files, and CI fails the build if they disagree —
see the *Versions agree* job. The mismatch is not cosmetic: the update dialog
compares the release tag against the Cargo version, while the updater compares
the manifest against the `tauri.conf.json` version, so drift makes the app offer
an update it then refuses to install, with nothing to tell the user why.

```bash
# 1. Bump the version. Edit these two by hand:
#      Cargo.toml                  [workspace.package] version
#      src-tauri/tauri.conf.json   version
# then let the tooling do the rest:
npm version <x.y.z> --no-git-tag-version   # package.json + package-lock.json
cargo check -p netdiag-core -p transom     # rewrites the versions in Cargo.lock

# 2. Verify before publishing anything
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo metadata --locked --format-version 1 >/dev/null
npm run typecheck && npm run lint

# 3. Ship
git commit -am "Release <x.y.z>"
git push origin main
git tag -a v<x.y.z> -m "Transom <x.y.z>"
git push origin v<x.y.z>
```

Pushing a `v*` tag runs the release workflow. `workflow_dispatch` with a version
input does the same thing and leaves the release as a draft.

Version numbers are compared numerically, not as strings — `1.10.0` is newer
than `1.9.0`, and `update.rs` has a test pinning exactly that.

## What CI does

1. Creates one draft release, so the four platform jobs have somewhere to upload
   without racing to create duplicates.
2. Builds macOS (Apple silicon and Intel), Linux and Windows, signing the
   updater artifacts for each.
3. Merges each platform's entries into `latest.json` on the release.
4. **Verifies `latest.json` covers all four platforms, and refuses to publish if
   it does not.** Each job rewrites that file by reading, merging and
   re-uploading it, so two finishing at the same instant can drop an entry — and
   a missing platform is invisible until nobody on it can update. Re-running the
   workflow regenerates the manifest.
5. Undrafts the release, at which point installs begin to see it.

The updater reads
`https://github.com/MarkoVcode/transom/releases/latest/download/latest.json`,
which resolves to the newest published, non-prerelease release.

## Checking a release

Confirm the published artifacts were signed by the key the app actually trusts:

```bash
man=$(curl -sL https://github.com/MarkoVcode/transom/releases/latest/download/latest.json)
pub=$(jq -r '.plugins.updater.pubkey' src-tauri/tauri.conf.json \
        | base64 -d | sed -n 2p | base64 -d | xxd -s 2 -l 8 -p)

for k in linux-x86_64 darwin-aarch64 darwin-x86_64 windows-x86_64; do
  sig=$(echo "$man" | jq -r --arg k "$k" '.platforms[$k].signature' \
          | base64 -d | sed -n 2p | base64 -d | xxd -s 2 -l 8 -p)
  printf "%-16s %s\n" "$k" "$([ "$sig" = "$pub" ] && echo OK || echo MISMATCH)"
done
```

A mismatch means CI signed with a different key from the one compiled into the
app, and no install will accept the release.

## Re-running a release for an existing tag

Don't, unless the release is still a draft. The workflow creates a release
unconditionally, and undrafting then fails with `already_exists: tag_name`
because a published release already holds that tag. The build jobs all succeed,
so it looks like a partial failure; what it leaves behind is a duplicate draft
that has to be deleted by ID, since two releases then share one tag and deleting
by tag may take the wrong one.

Cut a new version instead.

## Rotating the key

Only one public key is trusted at a time — the config field is a single value,
not a list — so rotation has to be rolled forward while the old key still works:

1. Generate a new keypair: `npx tauri signer generate -w <path>`.
2. Put the new public key in `src-tauri/tauri.conf.json`.
3. Replace the CI secret with the new private key.
4. Release. **That release is signed with the old key**, so existing installs
   accept it, and it carries the new public key into them.
5. Later releases are signed with the new key.

**Keep the old key until adoption is high.** Anyone who skips the transitional
release is stranded on the old public key and can only be recovered by a manual
reinstall. Destroying the old key immediately makes that permanent.

## What does not self-update

`.deb` and `.rpm` installs open the releases page instead. Those files belong to
the package manager, and applying an update through it needs root, which this app
promises never to require. On Linux only the AppImage updates in place — the app
decides by checking whether it is running from one.

Windows shows an administrator prompt on update, because the installer is
per-machine. Changing that to a per-user install would make updates silent, at
the cost of leaving existing per-machine installs behind.
