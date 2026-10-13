#!/usr/bin/env python3
"""Headless faster-whisper bridge used by MuxSift.

The script is intentionally independent of Buzz's Qt application. It runs in
Buzz's uv environment, so it reuses the already installed faster-whisper
package and locally downloaded models without modifying the Buzz checkout.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path


PROGRESS_PREFIX = "MUXSIFT_PROGRESS "


def report_progress(processed: float, total: float) -> None:
    print(
        PROGRESS_PREFIX
        + json.dumps(
            {"processed": max(0.0, processed), "total": max(0.0, total)},
            separators=(",", ":"),
        ),
        file=sys.stderr,
        flush=True,
    )


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Transcribe one local audio file")
    parser.add_argument("--input", required=True)
    parser.add_argument("--model", default="large-v3-turbo")
    parser.add_argument("--device", default="auto")
    parser.add_argument("--compute-type", default="default")
    parser.add_argument("--language", default="")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    audio = Path(args.input)
    if not audio.is_file():
        raise FileNotFoundError(f"audio input does not exist: {audio}")

    from faster_whisper import WhisperModel

    print(f"Buzz: loading model {args.model}", file=sys.stderr, flush=True)
    model = WhisperModel(
        args.model,
        device=args.device,
        compute_type=args.compute_type,
        local_files_only=True,
    )
    print("Buzz: transcribing audio", file=sys.stderr, flush=True)
    segments, info = model.transcribe(
        str(audio),
        language=args.language or None,
        vad_filter=True,
    )
    duration = float(info.duration or 0.0)
    report_progress(0.0, duration)
    output_segments = []
    for segment in segments:
        text = segment.text.strip()
        if text:
            output_segments.append(
                {"start": segment.start, "end": segment.end, "text": text}
            )
        report_progress(min(float(segment.end), duration), duration)
    if not output_segments:
        raise RuntimeError("no speech was recognized")
    report_progress(duration, duration)
    json.dump(
        {
            "language": info.language,
            "language_probability": info.language_probability,
            "segments": output_segments,
        },
        sys.stdout,
        ensure_ascii=False,
    )
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(f"Buzz local transcription failed: {error}", file=sys.stderr)
        raise SystemExit(1)
