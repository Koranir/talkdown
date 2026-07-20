# Whisper model provisioning research

Date checked: 2026-07-20

Talkdown’s downloadable default is the English whisper.cpp `base.en` GGML
model. The upstream
[`download-ggml-model.sh`](https://github.com/ggerganov/whisper.cpp/blob/master/models/download-ggml-model.sh)
lists `base.en` and resolves model assets from the official
[`ggerganov/whisper.cpp` Hugging Face repository](https://huggingface.co/ggerganov/whisper.cpp/tree/main).

The implementation pins repository revision
`5359861c739e955e79d9a303bcbc70fb988958b1` instead of downloading from a
moving `main` URL:

```text
https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-base.en.bin
```

At that revision, Hugging Face LFS metadata and the resolver headers agree on:

```text
filename: ggml-base.en.bin
size:     147964211 bytes
sha256:   a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002
```

These three values are one integrity tuple. If upstream weights are updated,
do not change only the URL: recheck the official repository revision, LFS OID,
resolver size, whisper-rs compatibility, and a real transcription fixture,
then update all values together.

## Implementation decisions

- `directories::ProjectDirs` supplies platform config/data roots instead of
  assuming Linux `$HOME` paths.
- `ureq` performs HTTPS on a dedicated blocking worker, never the iced update
  thread or CPAL callback.
- The worker writes `ggml-base.en.bin.part`, reports throttled byte progress,
  and checks cancellation between chunks.
- A download is installable only when byte count and SHA-256 both match.
- The temporary file is flushed and synced before rename. A failed or cancelled
  transfer removes it.
- If an invalid destination already exists, it is moved to an `.invalid`
  sibling before the verified replacement is installed, preserving recovery.
- Settings stages the verified path. The currently running speech worker is
  replaced only on Apply; closing Settings never silently changes it.
- `TALKDOWN_WHISPER_MODEL` remains the launch-time development/automation
  override, followed by the persisted path, then the installed default.

The normal test suite intercepts download events and uses in-memory readers for
success, truncation, checksum, progress, cancellation, and UI failure behavior.
It does not fetch 148 MB. Real download and native Whisper checks remain
explicit integration/smoke work.
