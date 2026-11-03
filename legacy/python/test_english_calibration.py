import json
from pathlib import Path
import unittest

import cv2
import numpy as np

import bot


ROOT = Path(__file__).resolve().parent


class EnglishCalibrationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        # Read the shared profile only: tests never load this PC's overrides or contact ADB.
        cls.config = bot.read_config(ROOT / 'config.english.json')
        cls.runner = bot.Bot(cls.config, ROOT, None)
        cls.fixtures = json.loads((ROOT / 'fixtures/english.json').read_text(encoding='utf-8'))

    def frame(self, name):
        return cv2.imread(str(ROOT / 'fixtures' / self.fixtures[name]['file']))

    def test_profile_validates_with_existing_farming_and_wall_guards(self):
        bot.validate_config(self.config, ROOT, 'both')
        bot.validate_walls(self.config, ROOT, ('home', 'builder'))
        spec = self.config['walls']['home']
        self.assertEqual((spec['gold_reserve'], spec['elixir_reserve'], spec['farm_threshold']),
                         (1_000_000, 1_000_000, 20_000_000))
        observed = set(self.config['calibration_observed_screens'])
        pending = set(self.config['calibration_pending_screens'])
        self.assertFalse(observed & pending)
        self.assertEqual(observed | pending, set(self.config['screens']))
        self.assertEqual(observed, {item['state'] for item in self.fixtures.values() if item['state']})

    def test_recognizes_captured_english_ui_without_account_identifiers(self):
        for name, item in self.fixtures.items():
            with self.subTest(name=name):
                if item['state']:
                    self.assertEqual(self.runner.detector.detect(self.frame(name)), item['state'])
                else:
                    with self.assertRaises(bot.UnknownScreen):
                        self.runner.detector.detect(self.frame(name))

    def test_unknown_and_dimmed_village_receive_no_inputs(self):
        for image in (np.zeros((720, 1280, 3), dtype=np.uint8), self.frame('home') // 2,
                      self.frame('builder') // 2):
            with self.assertRaises(bot.UnknownScreen):
                self.runner.detector.detect(image)

    def test_english_capacity_labels_read_and_do_not_match_closed_tooltips(self):
        for resource in ('gold', 'elixir'):
            with self.subTest(resource=resource):
                spec = self.config['capacity']['home'][resource]
                image = self.frame(f'tooltip_{resource}')
                self.assertTrue(self.runner.tooltip_open(image, spec))
                self.assertFalse(self.runner.tooltip_open(self.frame('home'), spec))
                self.assertEqual(bot.read_number(bot.crop(image, spec['max']), self.runner.templates(),
                                                 bot.FRACTION_MIN_SCORE), 26_000_000)

    def test_real_wall_prices_keep_reserves_and_require_the_correct_icon(self):
        for village, resource, price in (('home', 'gold', 2_500_000), ('home', 'elixir', 2_500_000),
                                         ('builder', 'gold', 240_000), ('builder', 'elixir', 400_000)):
            with self.subTest(village=village, resource=resource):
                prefix = 'wall' if village == 'home' else 'builder_wall'
                image = self.frame(f'{prefix}_confirm_{resource}')
                spec = self.config['walls'][village]
                self.assertEqual(self.runner.confirm_price(image, spec), price)
                self.assertTrue(self.runner.confirm_safe(image, spec, resource))
                other = 'elixir' if resource == 'gold' else 'gold'
                self.assertFalse(self.runner.confirm_safe(image, spec, other))
                self.assertTrue(self.runner.keeps_reserve(image, spec, price + 1_000_000, 1_000_000))
                self.assertFalse(self.runner.keeps_reserve(image, spec, price + 999_999, 1_000_000))
                x, y, w, h = spec['confirm']['icon']
                image[y:y+h, x:x+w] = 0
                self.assertFalse(self.runner.confirm_safe(image, spec, resource))

    def test_english_wall_rows_do_not_select_other_buildings(self):
        spec = self.config['walls']['home']
        self.assertEqual(self.runner.wall_rows(self.frame('build_list_scroll2'), spec),
                         [(3_500_000, 234), (2_500_000, 519)])
        self.assertFalse(self.runner.wall_rows(self.frame('build_list'), spec))

    def test_indonesian_profile_and_unchanged_icon_assets_are_reused(self):
        original = bot.read_config(ROOT / 'config.json')
        for name, index in (('home', 1), ('builder', 1), ('builder', 2), ('home_battle', 0),
                            ('build_list', 1), ('builder_cart', 1)):
            self.assertEqual(self.config['screens'][name]['anchors'][index],
                             original['screens'][name]['anchors'][index])
        self.assertEqual(self.config['walls']['home']['confirm']['icons'],
                         original['walls']['home']['confirm']['icons'])
        self.assertNotIn('english', original['screens']['home']['anchors'][0]['file'])


if __name__ == '__main__':
    unittest.main()
