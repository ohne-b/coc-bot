import os
import re
import sys


def valid_title(title: str) -> bool:
    return re.fullmatch(
        r'(feat|fix|docs|style|refactor|perf|test|build|ci|chore|revert)'
        r'(\([a-z0-9][a-z0-9._/-]*\))?!?: \S[^\r\n]*', title
    ) is not None


if __name__ == '__main__':
    if not valid_title(os.environ.get('PR_TITLE', '')):
        sys.exit('Use a conventional PR title, for example: fix(vision): reject unreadable prices')
