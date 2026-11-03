import unittest

from check_pr_title import valid_title


class ConventionalTitleTests(unittest.TestCase):
    def test_accepts_types_scopes_and_breaking_changes(self):
        for title in ('feat: add controls', 'fix(vision): reject unknown screens',
                      'refactor(core)!: replace the runner', 'chore!: retire Python',
                      'ci(deps): bump actions/checkout to v6'):
            with self.subTest(title=title):
                self.assertTrue(valid_title(title))

    def test_rejects_missing_fields_unknown_types_and_multiline_titles(self):
        for title in ('', 'update stuff', 'Feat: add controls', 'misc: update',
                      'fix: ', 'fix:  ', 'fix:missing space', 'feat(): add controls',
                      'fix: valid\nextra', 'fix: valid\n', 'fix: valid\rextra'):
            with self.subTest(title=title):
                self.assertFalse(valid_title(title))
