#!/usr/bin/env python3
"""Prepare project-owned Vietnamese speech locally; runtime only reads the resulting WAVs."""
import argparse
from pathlib import Path
import shutil
import subprocess
import tempfile
import wave


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, default=Path('assets/enrollment/vi-VN'))
    args = parser.parse_args()
    for program in ('espeak-ng', 'ffmpeg'):
        if shutil.which(program) is None:
            parser.error(f'{program} is required (install espeak-ng and ffmpeg)')
    phrases = {'intro': 'Mã kết nối của thiết bị là.'}
    phrases.update(dict(enumerate(('không', 'một', 'hai', 'ba', 'bốn', 'năm', 'sáu', 'bảy', 'tám', 'chín'))))
    args.output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='enrollment-assets-') as directory:
        for name, text in phrases.items():
            source = Path(directory) / f'{name}.wav'
            target = Path(directory) / f'{name}-24k.wav'
            subprocess.run(['espeak-ng', '-v', 'vi', '-s', '170', '-w', str(source), text], check=True, timeout=30)
            subprocess.run(['ffmpeg', '-nostdin', '-loglevel', 'error', '-y', '-i', str(source),
                            '-af', 'silenceremove=start_periods=1:start_threshold=-50dB,areverse,silenceremove=start_periods=1:start_threshold=-50dB,areverse',
                            '-ar', '24000', '-ac', '1', '-c:a', 'pcm_s16le', str(target)], check=True, timeout=30)
            with wave.open(str(target)) as audio:
                maximum = 6 if name == 'intro' else 1
                if audio.getframerate() != 24000 or audio.getnchannels() != 1 or audio.getsampwidth() != 2 or not 0 < audio.getnframes() <= 24000 * maximum:
                    raise ValueError(f'{name}: clip does not meet enrollment bounds')
            shutil.copyfile(target, args.output / f'{name}.wav')
    print(f'Prepared 11 clips in {args.output}. Listen to them before deployment.')


if __name__ == '__main__':
    main()
