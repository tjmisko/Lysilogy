"""Exercise the exact trusted Device adapter with authored, offline fake pixels."""
import hashlib
import json
from pathlib import Path
import subprocess
import unittest

ROOT=Path(__file__).resolve().parents[2]

class MaskAdapterTests(unittest.TestCase):
    def should_preserve_frozen_opacity_cases_when_the_actual_adapter_reads_synthetic_images(self):
        result=subprocess.run(['node',str(ROOT/'scripts/eval/mask-fixture-adapter.js'),str(ROOT/'eval/fixtures/mask-support.json'),str(ROOT/'src/source_index/graphics/masks.js')],capture_output=True,check=True,timeout=10)
        expected=json.loads((ROOT/'eval/fixtures/mask-events.json').read_bytes())
        actual=json.loads(result.stdout)
        self.assertEqual(actual,expected)
        self.assertEqual(len(actual['cases']),43)
        self.assertEqual(actual['independent_packet_sha256'],hashlib.sha256((ROOT/'eval/fixtures/mask-support.json').read_bytes()).hexdigest())
        cases={case['id']:case for case in actual['cases']}
        for suffix in ('short_array','out_of_range_sample','negative_sample','float_sample','boolean_sample','more_than_one_component'):
            row=cases['strict_decoded_array_and_transform/'+suffix]['receipt']['operations'][0]
            self.assertIsNotNone(row['error'],suffix)
        sample=cases['partial_alpha_corner_counts_as_support']['receipt']['operations'][0]['mask']['sample_hex_rows'][0]
        self.assertEqual(sample,'00000001')

def load_tests(loader, tests, pattern):
    return unittest.TestSuite(MaskAdapterTests(name) for name in dir(MaskAdapterTests) if name.startswith('should_'))

if __name__=='__main__': unittest.main()
