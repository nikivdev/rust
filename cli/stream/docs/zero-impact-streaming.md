# Zero-Impact macOS to Linux Streaming

`stream` is a CLI for headless, daemon-mode streaming from macOS to a Linux host using ffmpeg and SRT. This guide covers how to achieve zero performance impact on your workflow.

## Why Zero-Impact Matters

Traditional streaming solutions like OBS can cause:
- Input lag (mouse/keyboard feel sluggish)
- UI stuttering during encoding
- Higher CPU temperatures and fan noise
- Reduced battery life on laptops

The `stream` CLI avoids these by:
1. **Hardware encoding only** - VideoToolbox uses Apple Silicon's dedicated media engine
2. **Lowest process priority** - ffmpeg runs at nice 19 (idle priority)
3. **Minimal probing** - Instant startup without analyzing input
4. **SRT transport** - Efficient, low-overhead network protocol

## Quick Start

```bash
# Initialize config
stream config init

# Edit config with your settings
$EDITOR ~/Library/Application\ Support/stream/config.toml

# Start streaming
stream start --profile zero-impact

# Or run as daemon (auto-restarts on failure)
stream daemon --profile zero-impact
```

## The Zero-Impact Profile

Add this to your config for minimal system impact:

```toml
[profiles.zero-impact]
description = "Zero-impact streaming"

[profiles.zero-impact.remote]
host = "your-linux-host"
user = "stream"
tmux_session = "streamd"
ingest_port = 6000

[profiles.zero-impact.remote.runner]
type = "ffmpeg"
output = "/path/to/output.ts"
copy_video = true
copy_audio = true

[profiles.zero-impact.local]
ffmpeg_path = "/opt/homebrew/bin/ffmpeg"
fps = 60
video_bitrate = "6000k"
maxrate = "8000k"
bufsize = "12000k"
# Key settings for zero-impact:
nice = 19           # Lowest priority
probesize = 32      # Skip input analysis
analyzeduration = 0 # Instant start

[profiles.zero-impact.local.capture]
type = "avfoundation"
video_device = "1"
audio_device = "none"
pixel_format = "uyvy422"
capture_cursor = true

[profiles.zero-impact.local.encoder]
type = "h264_videotoolbox"
quality = "Speed"   # Less GPU work
allow_sw = false    # Never use CPU

[profiles.zero-impact.local.transport]
type = "srt"
latency_ms = 20
```

## Key Settings Explained

### Process Priority (`nice`)

```toml
nice = 19  # Range: -20 (highest) to 19 (lowest)
```

- `0` = Normal priority (default for most apps)
- `10` = Low priority (stream default)
- `19` = Idle priority (only runs when system is idle)

For truly zero impact, use `nice = 19`. The encoding will still complete in real-time because VideoToolbox hardware encoding is fast enough.

### VideoToolbox Encoding

```toml
[encoder]
type = "h264_videotoolbox"
quality = "Speed"
allow_sw = false
```

VideoToolbox uses Apple Silicon's dedicated media engine:
- **M1/M2/M3** have hardware H.264/HEVC encoders
- Zero CPU cycles for encoding
- Negligible power consumption
- `allow_sw = false` ensures it never falls back to CPU

Quality options:
- `"Speed"` - Faster, larger files, less GPU work (recommended)
- `"Quality"` - Better compression, more GPU compute
- `"Balanced"` - Middle ground

### Instant Startup

```toml
probesize = 32
analyzeduration = 0
```

Normal ffmpeg spends time analyzing the input to detect format, codecs, etc. Since avfoundation capture is predictable, we skip this:
- `probesize = 32` - Minimal bytes to read
- `analyzeduration = 0` - No analysis delay

Result: Stream starts in <100ms instead of 1-2 seconds.

### No Scaling

For zero CPU impact, avoid software scaling:

```toml
# DON'T do this (uses CPU):
scale_filter = "scale=-2:1080:flags=lanczos"

# DO this instead (hardware scaling):
scale_filter = "scale_vt=-2:1080"

# OR best: no scaling at all
# (just remove scale_filter)
```

If you must scale, use `scale_vt` for VideoToolbox hardware scaling.

## Daemon Mode

Run streaming as a background daemon with auto-restart:

```bash
stream daemon --profile zero-impact
```

Options:
- `--restart-delay 5` - Wait 5 seconds before restarting (default)
- `--max-restarts 10` - Stop after 10 restarts (0 = unlimited)
- `--skip-remote` - Don't manage remote receiver

The daemon:
- Handles SIGTERM/SIGINT gracefully
- Restarts ffmpeg if it crashes
- Keeps the remote tmux session alive
- Logs to `~/Library/Application Support/stream/logs/`

## Monitoring

```bash
# Check status
stream status

# Check status including remote tmux
stream status --remote

# View logs
tail -f ~/Library/Application\ Support/stream/logs/stream-*.log
```

## Network Tuning

### SRT Settings

```toml
[transport]
type = "srt"
latency_ms = 20    # Lower = less delay, more sensitive to jitter
packet_size = 1316 # MTU-safe packet size
```

SRT (Secure Reliable Transport) advantages over RTMP/UDP:
- Built-in error correction
- Handles packet loss gracefully
- Low latency (20-50ms achievable)
- Encrypted (optional passphrase)

### Bandwidth

For 1080p60:
- `6000k` - Good quality, moderate bandwidth (~50 Mbps)
- `9000k` - High quality (~75 Mbps)
- `12000k` - Near-lossless (~100 Mbps)

For 4K60:
- `15000k` - Good quality
- `25000k` - High quality
- `40000k` - Near-lossless

## Remote Receiver Setup

On your Linux host, run ffmpeg to receive the SRT stream:

```bash
# Simple file recording
ffmpeg -i "srt://0.0.0.0:6000?mode=listener" -c copy output.ts

# Or use the stream CLI's remote runner
# (automatically started in tmux via SSH)
```

The stream CLI manages this automatically via SSH + tmux. Just configure:

```toml
[remote]
host = "linux-box"
user = "stream"
tmux_session = "streamd"
ingest_port = 6000

[remote.runner]
type = "ffmpeg"
output = "/path/to/recording.ts"
copy_video = true
copy_audio = true
```

## Troubleshooting

### "ffmpeg not found"

Install ffmpeg with VideoToolbox support:

```bash
brew install ffmpeg
```

### "VideoToolbox encoder not available"

Ensure you're on Apple Silicon (M1/M2/M3) or an Intel Mac with hardware encoding support. Check:

```bash
ffmpeg -encoders | grep videotoolbox
```

### High CPU despite settings

1. Check you're not using software scaling
2. Verify `allow_sw = false`
3. Check nice value: `ps -o pid,ni,comm | grep ffmpeg`

### Stream stuttering

1. Increase `latency_ms` (try 50-100)
2. Reduce bitrate
3. Check network stability: `ping your-linux-host`

### Finding video/audio devices

```bash
ffmpeg -f avfoundation -list_devices true -i ""
```

Device indices (e.g., "1" for video, "0" for audio) go in your config.

## Performance Comparison

| Solution | CPU Impact | Input Lag | Startup Time |
|----------|-----------|-----------|--------------|
| OBS (x264) | High | Noticeable | 2-3s |
| OBS (VideoToolbox) | Low | Minor | 2-3s |
| stream CLI (default) | Very Low | None | <1s |
| stream CLI (zero-impact) | Zero | None | <100ms |

The zero-impact profile is designed for 24/7 background streaming without any perceptible effect on your workflow.
