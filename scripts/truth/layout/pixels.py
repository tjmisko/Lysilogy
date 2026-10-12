"""Bounded lossless PPM decoding and exact whole-page correspondence."""
import hashlib

from policy import Refused

MAX_RASTER_BYTES = 16 * 1024 ** 2
MAX_SIDE = 8192


def decode_ppm(raw):
    if len(raw) > MAX_RASTER_BYTES or not raw.startswith(b"P6") or len(raw) < 3 or raw[2] not in b" \t\r\n":
        raise Refused("unsupported or oversized page raster")
    position, tokens = 2, []
    while len(tokens) < 3:
        while position < len(raw) and raw[position] in b" \t\r\n":
            position += 1
        if position >= min(len(raw), 4096):
            raise Refused("incomplete or oversized raster header")
        if raw[position] == 35:
            end = raw.find(b"\n", position)
            if end < 0 or end > 4096:
                raise Refused("unbounded raster comment")
            position = end + 1
            continue
        start = position
        while position < min(len(raw), 4096) and 48 <= raw[position] <= 57:
            position += 1
        token = raw[start:position]
        if not token or len(token) > 8:
            raise Refused("invalid raster dimension")
        tokens.append(int(token))
    if position >= len(raw) or raw[position] not in b" \t\r\n":
        raise Refused("raster header lacks its final separator")
    # The raw format consumes exactly one separator, not arbitrary whitespace:
    # a pixel may legitimately start with space, newline, '#', or zero.
    position += 1
    width, height, maximum = tokens
    if maximum != 255 or not 0 < width <= MAX_SIDE or not 0 < height <= MAX_SIDE:
        raise Refused("unsupported raster sample depth or dimensions")
    expected = width * height * 3
    if expected > MAX_RASTER_BYTES or len(raw) - position != expected:
        raise Refused("raster payload size does not match its dimensions")
    pixels = raw[position:]
    return {"width": width, "height": height, "channels": 3,
            "pixel_sha256": hashlib.sha256(pixels).hexdigest()}, pixels


def compare_pages(original, rebuilt):
    original_info, original_pixels = decode_ppm(original)
    rebuilt_info, rebuilt_pixels = decode_ppm(rebuilt)
    return {"original": original_info, "rebuilt": rebuilt_info,
            "exact_whole_page": original_info == rebuilt_info and original_pixels == rebuilt_pixels}


def whole_document(pages, original_count, rebuilt_count):
    if type(original_count) is not int or type(rebuilt_count) is not int or not 0 < original_count <= 8:
        raise Refused("invalid original document size")
    expected = {(page, dpi) for page in range(1, original_count + 1) for dpi in (96, 192)}
    observed = [(item["page"], item["dpi"]) for item in pages]
    if len(set(observed)) != len(observed) or set(observed) != expected:
        raise Refused("incomplete or repeated page correspondence inventory")
    return original_count == rebuilt_count and all(item["exact_whole_page"] is True for item in pages)
