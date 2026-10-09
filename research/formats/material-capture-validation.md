# Deterministic material captures, JC2-088

The material viewer accepts mouse/pan/scroll input during ordinary use. Earlier
JC2-024 captures with identical arguments and model bounds had one whole-scene
framing mismatch. Its cause remains unconfirmed; the original PNG/logs are retained
in `target/foliage49/`. Repeating that pair produced identical images.

`--material-capture` requires `--material-check` and `--screenshot`. It disables
orbit/pan/zoom/reframe input while retaining the configured camera angles and the
normal one-time model framing. At screenshot request time it writes a sibling
`*.camera.json` containing the actual camera target, angles, distance, transform,
projection, viewport, selected models and recursive asset readiness. Camera input
remains locked while the GPU capture completes. The receipt does not establish
that every shader has finished compiling or that a scene matches the retail game.

`scripts/check-material-capture.ps1` launches a private validated executable,
records its SHA256 and exact arguments, retains both logs and checks successful
exit, fresh PNG/metadata, locked camera and asset readiness. It requires a fresh
output directory. With `-CompareReceipt`, it rejects a different final camera,
viewport or model selection before any image comparison is accepted. Light,
normal-map and depth-prepass settings are recorded and may deliberately differ.
The `-CheckReceipt` mode compares two existing receipts without a game launch.

```powershell
& scripts/check-material-capture.ps1 -Executable $privateExe -GameDirectory $game -OutputDirectory $main
& scripts/check-material-capture.ps1 -Executable $privateExe -GameDirectory $game -OutputDirectory $depth -DepthPrepass -CompareReceipt "$main/receipt.json"
& scripts/check-material-capture.ps1 -CheckReceipt "$depth/receipt.json" -CompareReceipt "$main/receipt.json"
```

Source syntax/format checks passed. Compilation and repeated GPU/camera comparison
acceptance are pending the shared build slot. Interactive viewer controls remain
available when the explicit capture flag is absent.
