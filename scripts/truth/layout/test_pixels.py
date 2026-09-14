"""No fuzzy similarity, crops or page-number matches can establish fidelity."""
import unittest
from pixels import compare_pages, decode_ppm, whole_document
from policy import Refused


def ppm(pixels, width=2, height=1, comment=b""):
    return b"P6\n" + comment + f"{width} {height}\n255\n".encode() + pixels


class PixelTests(unittest.TestCase):
    def should_compare_decoded_pixels_when_harmless_headers_differ(self):
        pixels = b"\n #\x00\xff\x01"
        self.assertTrue(compare_pages(ppm(pixels), ppm(pixels, comment=b"# independent header\n"))["exact_whole_page"])
        self.assertEqual(decode_ppm(ppm(pixels))[1], pixels)

    def should_reject_any_changed_pixel_when_full_page_fidelity_is_claimed(self):
        first = ppm(b"\x00" * 6)
        for position in range(6):
            changed = bytearray(6)
            changed[position] = 1
            self.assertFalse(compare_pages(first, ppm(bytes(changed)))["exact_whole_page"])
        self.assertFalse(compare_pages(first, ppm(b"\x00" * 6, width=1, height=2))["exact_whole_page"])

    def should_reject_invalid_lengths_and_depths_when_decoding_untrusted_rasters(self):
        for raw in (b"P6\n99999999 2\n255\nx", b"P6\n2 1\n256\n" + b"x" * 6,
                    ppm(b"x" * 5), ppm(b"x" * 7), b"P6\n-1 2\n255\nx", b"P6\n# missing newline", b"P62 1\n255\nxxxxxx"):
            with self.subTest(raw=raw), self.assertRaises(Refused):
                decode_ppm(raw)

    def should_require_both_resolutions_and_all_pages_when_qualifying_a_document(self):
        pages = [{"page": page, "dpi": dpi, "exact_whole_page": True} for page in (1, 2) for dpi in (96, 192)]
        self.assertTrue(whole_document(pages, 2, 2))
        self.assertFalse(whole_document(pages, 2, 3))
        with self.assertRaises(Refused):
            whole_document(pages[:3], 2, 2)
        with self.assertRaises(Refused):
            whole_document(pages + [pages[0]], 2, 2)
        pages[2]["exact_whole_page"] = False
        self.assertFalse(whole_document(pages, 2, 2))


if __name__ == "__main__":
    loader = unittest.TestLoader()
    loader.testMethodPrefix = "should_"
    unittest.main(testLoader=loader, verbosity=2)
