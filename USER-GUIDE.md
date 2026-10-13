# MuxSift User Guide

Extract the Linux x86_64 archive, then run `./release/start-tui.sh` or `./release/muxsift tui`. Its `bin` directory already contains aria2 and FFmpeg, so testers do not need to install either tool. Press `a` to add a BV URL or b23.tv short link, choose pages with `Space`, and confirm with `Enter`.

```bash
./release/muxsift login
./release/muxsift BV1xxxx --quality 1080p -o "$HOME/Videos"
./release/muxsift BV1xxxx --audio-only
./release/muxsift BV1xxxx --subtitle --subtitle-format srt
./release/muxsift BV1xxxx --transcribe --transcript-format srt
./release/muxsift BV1xxxx --pages all
```

In TUI: `a` add, `Space` mark, `S` start marked, `Enter` start current, `x` cancel, `P` pause/resume, `r` resume a failed/cancelled task from its last successful stage, `d` delete, `m` mode, `b` download backend, `B` Bcut/Buzz ASR backend, `c` paths/Bcut cooldown, `C` AI settings, `g` language, and `h` help.

## Local video and audio

The `local` command extracts audio with FFmpeg, transcribes it to SRT, and sends the SRT to the configured AI provider to create same-name Markdown notes:

```bash
./release/muxsift local ./meeting.mp4 --asr bcut -o ./notes
./release/muxsift local ./meeting.wav --asr buzz --buzz-project /path/to/buzz -o ./notes
```

For Bcut, run `login` first; the audio is uploaded to Bcut. Buzz processes audio locally. Enable AI in the JSON config, set `ai.model` and `ai.base_url`, and export the variable named by `ai.api_key_env` (by default `OPENAI_API_KEY`). Use `--transcript-only` when you only want SRT, and `--resume` after a failure to reuse extracted audio or an existing SRT. Existing output files are not overwritten.

In TUI, press `a` and enter a local video, audio file, or folder. A folder is scanned recursively for nonempty common media files and opens a directory tree picker. Use arrows to move, Space to select a file or folder, `/` to search, `A` to select all, `E` to fold or unfold, and Enter to confirm. Each selected file becomes a task; press `A` then `S` to start them all. Batch results go under a folder of the same name in the configured transcript output directory, preserving subdirectories. Files with the same stem and different extensions get separate output subfolders. Configure Bcut/Buzz and AI with `c`; press `r` on a failed task to resume.

If download, FFmpeg, ASR, or AI fails, verified intermediate files are retained under a hidden `.muxsift-work` directory. Fix the cause and run the same CLI command with `--resume`, or press `r` on the failed TUI task. For example, an ASR retry reuses the saved M4A, and an AI retry reuses the saved SRT. Successful completion removes the checkpoint automatically. Changing between Bcut and Buzz is allowed because both consume the same saved M4A; incompatible BV/page or media-stream checkpoints are rejected.

Use `./release/muxsift config show` to locate the JSON configuration and `config init` to create it. Credentials and API keys must not be shared.

AI post-processing is optional. Press `c` in the TUI to open one settings window. Tab or Left/Right switches between General, Transcription, AI, and Output Paths; Up/Down selects a field, and Enter edits or toggles it. The Transcription section includes the Bcut/Buzz backend, Buzz project directory, `uv`, model, and related options. Press `C` to open the AI section directly, or `t` there to test the connection. In the prompt editor, Enter adds a newline, `F2` saves, and `Esc` discards unsaved edits and closes settings. Set the key in the launching shell, then transcribe as SRT. The original `.srt` is retained and a same-name `.md` is created. The SRT is sent to your configured AI provider, so review its privacy policy first.

If the SRT already exists, repeating that transcript task asks whether to skip the download and ASR steps and send the existing SRT to AI. Enter `y` in the CLI, or choose **Use existing SRT** in the TUI. The safe default cancels the task. This option requires enabled AI configuration and is unavailable when the same-name `.md` already exists; existing files are never overwritten.

## Runtime tools and configuration

The package includes `bin/aria2c` and `bin/ffmpeg`. `--aria2 /path/to/aria2c`
and `--ffmpeg /path/to/ffmpeg` select other executables. If either bundled
program is missing, obtain a complete release archive; for a source checkout,
run `sudo apt install aria2 ffmpeg` and pass their paths. `config show` prints the JSON config path under the XDG config directory.
Login credentials are stored alongside it and are never included in the archive.

Buzz can use a downloaded faster-whisper model locally without uploading audio:

```bash
./release/muxsift BV1h3hb6jELV --transcribe --asr buzz \
  --buzz-project /home/you/Projects/buzz --buzz-uv /home/you/.local/bin/uv \
  --buzz-model large-v3-turbo --buzz-device cuda --buzz-compute-type float16
```

The same values can be saved under `asr` in the JSON config. CLI and TUI display live Buzz recognition percentage, processed/total media time, and ETA. Buzz jobs run one at a time to avoid loading several large models into GPU memory.

Maintainers can download the pinned portable tools and build the archive with
`./scripts/fetch-linux-tools.sh` followed by `./scripts/package-linux.sh`.
