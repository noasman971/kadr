# Test fixtures

Test videos are **generated, not committed**. Run:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/generate-test-fixtures.ps1
```

They are written to `tests/fixtures/generated/` (ignored by Git).

| File | Content | Used to test |
|---|---|---|
| `counter_1080p30_beep.mp4` | 5 s, 1920×1080, 30 fps, frame counter, 1 kHz beep every second | Decoding, frame-accurate seeking, audio/video sync |
| `colour_bars_720p25_tone.mp4` | 3 s, 1280×720, 25 fps, SMPTE bars, 440 Hz tone | A different frame rate, colour accuracy |
| `vertical_1080x1920_30_no_audio.mp4` | 3 s, 1080×1920 (9:16), 30 fps, no audio | Vertical video, files without audio |
| `moving_target_720p30.mp4` | 6 s, 1280×720, 30 fps, textured square moving right and bobbing, passing behind a black bar around t = 4 s | Region tracker, target lost and found again |
| `tone_silence_tone_360p30.mp4` | 6 s, 640×360, 30 fps, 440 Hz tone with silence from 2 s to 4 s | Silence detection and removal |

To add a fixture, add a `New-TestFixture` call to the script and a line to this table.
Keep fixtures small (a few seconds) so CI stays fast.
