Place the official Windows `yt-dlp.exe` beside the packaged `AutoFox.exe`.

The executable is intentionally not committed to the source repository. Record its version and license notice in the release artifacts.

## A JavaScript runtime is required for YouTube

yt-dlp needs a JS runtime to descramble the `n` query parameter on YouTube
media URLs. Without one it still probes and still returns a format list, so the
failure is invisible until a download starts: every CDN URL answers
`HTTP 403 Forbidden`. yt-dlp reports this only as a warning on stderr
(`No supported JavaScript runtime could be found`).

Verified on 2026-09-26 with yt-dlp 2026.08.19: the same URL that returned 403
returned `HTTP 206` once `deno.exe` was on `PATH`.

Ship `deno.exe` beside `yt-dlp.exe`, or accept that YouTube downloads fail on
machines without a runtime. Only `deno` is enabled by default; other runtimes
need `--js-runtimes RUNTIME[:PATH]`.
