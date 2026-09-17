# Bundled verification tools

Roch Microcode itself is licensed GPL-3.0-or-later; see `LICENSE`. The third-party components below retain their own licenses and copyright notices.

Roch Microcode distributes these tools without source modifications. Their license notices are included in `THIRD-PARTY/` and in the embedded verification archive.

| Component | Version | Upstream |
| --- | --- | --- |
| UEFIExtract | NE A75, Windows x64 | https://github.com/LongSoft/UEFITool/releases/tag/A75 |
| MCExtractor and database | 1.104.0 / r352 | https://github.com/platomav/MCExtractor/tree/r352 |
| CPython embedded runtime | 3.11.9, Windows x64 | https://www.python.org/ftp/python/3.11.9/ |
| colorama | 0.4.6 | https://pypi.org/project/colorama/0.4.6/ |
| PLTable | 1.1.0 | https://pypi.org/project/PLTable/1.1.0/ |

The Python runtime is private to the app and does not install or alter the user's Python environment. UEFIExtract reads firmware trees; MCExtractor reads and extracts microcode in private working directories. Neither is used to flash the motherboard. Automatic verifier update checks are disabled.
