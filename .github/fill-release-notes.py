"""Fill @@VER@@ and @@SHA@@ in the release notes template."""
import os
import pathlib
import sys

ver, dest, template, sums = sys.argv[1:]
text = pathlib.Path(template).read_text()
text = text.replace("@@VER@@", ver).replace("@@SHA@@", pathlib.Path(sums).read_text().rstrip())
repo = os.environ["GITHUB_REPOSITORY"]
sha = os.environ["GITHUB_SHA"]
text += f"\nWhat changed: [CHANGELOG.md](https://github.com/{repo}/blob/{sha}/CHANGELOG.md).\n"
pathlib.Path(dest).write_text(text)
