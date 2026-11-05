# Deferred JavaScript PDF navigation (experimental branch)

This branch is **not** a new implementation of JavaScript links. It extends the
still-open [upstream PR #2081](https://github.com/storytold/pdfcraft/pull/2081)
with deferred `app.openDoc` references and explicit remembered consent.

## Behavior

* `app.openDoc('./other.pdf', this)` returns a **deferred proxy** supporting
  `pageNum` (zero-based). The proxy does not contain file bytes or permission
  to open a file, and is not yet a complete Acrobat Doc object.
* Each call owns its own pending `OpenDoc { path, page }` viewer effect.
  Consecutive opens are queued independently; the second isn't dropped while
  the first file waits for an Allow/Cancel decision.
* Both relative and absolute local PDF links require a prompt by default
  (consistent with [PR #2152](https://github.com/storytold/pdfcraft/pull/2152)).
* A user may tick **Remember this PDF link** at approval time. Saved grants
  are scoped by absolute source path, SHA-256 of *the opened source bytes*, and
  resolved absolute target path. Editing and reopening the source file changes
  its digest and invalidates the grant. Merely clicking/opening never grants
  lasting permission. The Preferences dialog can delete all remembered grants.
* No target-file I/O takes place while building the request or deciding whether
  it is trusted. Only the existing vetted local PDF opener may read the target,
  after an explicit approval or a matching remembered grant.
* A maximum of 16 script requests may be queued at once; at most 128 grants
  are retained in settings. A source PDF larger than 32 MiB or without an
  absolute source path can still be opened with consent, but can't be remembered.

## Security boundaries and caveats

* Trusting a path grants permission to re-open the **path**, not to launch
  executables or to follow arbitrary URLs. The linked PDF is opened in PdfCraft.
* The grant does not identify the target file's content, and the target path
  could point to a different PDF after external filesystem changes (including
  symlink changes). The UI must show the full resolved path.
* Remembered trust is only a local desktop preference. A JSON settings restore
  validates the entry count, absolute paths, string lengths and lowercase
  hexadecimal digest before accepting any entry.
* A document is only opened when its source is still present in the session.
  A queued request from a closed source is discarded.
* Automatic opening of a *known* local PDF is intentional only after an
  affirmative remembered grant; it is not a sandbox escape.
* This branch is based on PR #2081's head commit
  `c47df1c3464751b9faca241a3807ac211239092c` and is not
  independently mergeable into upstream `main` without the parent PR.
* **Known limitations:** no complete Acrobat Doc methods beyond `pageNum`,
  no persistent per-link permission editor beyond Forget all, no translated
  strings for the newly added controls, and no automated evaluation of whether
  the target PDF later changes identity.

## Local verification required

The branch was edited remotely where the Rust toolchain is not available.
**No successful build, fmt, clippy or Rust test run is claimed.** In a
normal Rust 1.95+ checkout:

```bash
git fetch origin feat/acrobat-open-doc-bridge
git switch --track origin/feat/acrobat-open-doc-bridge
cargo fmt --all
cargo fmt --all -- --check
cargo test -p pdfcraft-js -j 2 two_open_doc_handles_keep_independent_page_destinations
cargo test -p pdfcraft-ui-egui -j 2 local_pdf_trust_tests
cargo test -p pdfcraft-ui-egui -j 2 multiple_open_doc_requests_preserve_destinations_and_consent
cargo clippy -p pdfcraft-js -p pdfcraft-engine -p pdfcraft-ui-egui -j 2 --all-targets -- -D warnings
cargo xtask layers
cargo xtask assets
```

For manual QA, keep `Naviga nelle Bibbie.pdf`, `nwt_I.pdf` and
`TNM con Rif e Note (Rbi8).pdf` in the same directory. Click a
JavaScript link; approve both dialogs with Remember selected. Repeat the
action and verify that both documents open at the requested pages without
new prompts; edit the source PDF, reopen, and verify the prompts return.
Run the same test once with JavaScript disabled.

No pull request should be submitted until builds, tests and cross-platform
review have passed.
