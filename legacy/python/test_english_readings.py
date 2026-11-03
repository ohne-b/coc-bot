import copy
import json
from pathlib import Path
import unittest

import cv2
import numpy as np

import bot


ROOT = Path(__file__).resolve().parent


class EnglishReadingTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.manifest = json.loads((ROOT / 'fixtures/english-readings.json').read_text())
        cls.cases = {case['id']: case for case in cls.manifest['fixtures']}
        cls.config = bot.read_config(ROOT / 'config.json')
        cls.runner = bot.Bot(cls.config, ROOT, None)

    def frame(self, case):
        return cv2.imread(str(ROOT / 'fixtures' / case['file']))

    def read(self, image, spec):
        patch = bot.crop(image, spec['roi'])
        reader = spec['reader']
        if reader.startswith('green_'):
            hsv = cv2.cvtColor(patch, cv2.COLOR_BGR2HSV)
            patch = cv2.cvtColor(cv2.inRange(hsv, (35, 120, 200), (85, 255, 255)), cv2.COLOR_GRAY2BGR)
        if reader in ('percent', 'green_percent'):
            return bot.read_percent(patch, self.runner.templates())
        if reader == 'trailing_number':
            return bot.read_trailing_number(patch, self.runner.templates())
        if reader == 'wall_row_price':
            x, y, w, h = spec['roi']
            return bot.row_price(image, [x, 0, w, h], y, self.runner.templates())
        if reader == 'gold_stars':
            return bot.earned_stars(image, spec['slots'])
        if reader == 'silver_stars':
            return sum(bot.white_pixels(image, slot) > 1000 for slot in spec['slots'])
        return bot.read_number(patch, self.runner.templates())

    def test_numeric_annotations_preserve_geometry_and_explicit_missing_values(self):
        self.assertEqual(self.manifest['resolution'], [1280, 720])
        self.assertEqual(self.manifest['dpi'], 240)
        for case in self.cases.values():
            with self.subTest(case=case['id']):
                image = self.frame(case)
                self.assertEqual(image.shape, (720, 1280, 3))
                retained = np.zeros((720, 1280), dtype=bool)
                for region in case['retained_regions']:
                    x, y, w, h = bot.rectangle(region, [1280, 720])
                    retained[y:y+h, x:x+w] = True
                self.assertFalse(image[~retained].any())
                if case['kind'] != 'wall_confirmation':
                    self.assertFalse(image[:68, :280].any())  # Modal wall UI itself covers this header.
                for name, spec in case['readings'].items():
                    self.assertIn(spec['status'], ('observed', 'unavailable', 'unreadable'))
                    if spec['status'] == 'unavailable':
                        self.assertIsNone(spec['value'])
                        self.assertIsNone(spec['roi'])
                        self.assertTrue(spec['reason'])
                        continue
                    x, y, w, h = bot.rectangle(spec['roi'], [1280, 720])
                    self.assertTrue(retained[y:y+h, x:x+w].all(), (case['id'], name))
                    if spec['status'] == 'observed':
                        self.assertIs(type(spec['value']), int)
                        self.assertGreaterEqual(spec['value'], 0)
                    else:
                        self.assertIsNone(spec['value'])

    def test_captured_numeric_readings_with_existing_confidence_checks(self):
        for case in self.cases.values():
            image = self.frame(case)
            for name, spec in case['readings'].items():
                if spec['status'] == 'unavailable' or spec.get('legacy_ocr_limit'):
                    continue
                with self.subTest(case=case['id'], reading=name):
                    if spec['status'] == 'unreadable':
                        with self.assertRaises(bot.BotError):
                            self.read(image, spec)
                    else:
                        self.assertEqual(self.read(image, spec), spec['value'])

    @unittest.expectedFailure
    def test_legacy_hud_reader_preserves_the_leading_gold_digits(self):
        case = self.cases['home-gold-tooltip']
        self.assertEqual(self.read(self.frame(case), case['readings']['balance.gold']), 7_099_808)

    def matching_layouts(self, image, layouts):
        return [layout['name'] for layout in layouts if all(
            bot.match_min(bot.crop(image, guard['region']), cv2.imread(str(ROOT / guard['file'])))
            <= guard['max_score'] for guard in layout['guards'])]

    def test_result_variants_require_exactly_one_set_of_independent_guards(self):
        layouts = [layout for state in ('home_result', 'builder_result')
                   for layout in self.config['screens'][state]['result_readings']]
        for layout in layouts:
            self.assertGreaterEqual(len(layout['guards']), 2)
            boxes = []
            for guard in layout['guards']:
                self.assertLessEqual(guard['max_score'], 0.05)
                self.assertTrue(all(type(scale) is float for scale in guard['scales']))
                x, y, w, h = bot.rectangle(guard['region'], [1280, 720])
                for a, b, c, d in boxes:
                    self.assertFalse(x < a+c and a < x+w and y < b+d and b < y+h)
                boxes.append((x, y, w, h))
        for case in self.cases.values():
            with self.subTest(case=case['id']):
                found = self.matching_layouts(self.frame(case), layouts)
                self.assertEqual(found, [case['layout']] if case['kind'] == 'battle_result' else [])
        victory = self.frame(self.cases['home-victory'])
        self.assertEqual(len(self.matching_layouts(victory, layouts + [copy.deepcopy(layouts[1])])), 2)
        self.assertFalse(self.matching_layouts(np.zeros_like(victory), layouts))

    def test_bonus_and_unavailable_resources_cannot_be_silently_added_or_zeroed(self):
        variants = self.config['screens']['home_result']['result_readings']
        victory = next(layout for layout in variants if layout['name'] == 'home_three_resources')
        self.assertTrue(victory['bonus'])
        self.assertNotIn('bonus_included', victory)
        self.assertNotIn('dark', variants[0]['loot'])
        builder = self.config['screens']['builder_result']['result_readings'][0]
        self.assertEqual(set(builder['loot']), {'gold'})
        self.assertEqual(self.cases['home-defeat']['readings']['loot.elixir']['value'], 0)
        self.assertIsNone(self.cases['home-defeat']['readings']['loot.dark']['value'])


if __name__ == '__main__':
    unittest.main()
