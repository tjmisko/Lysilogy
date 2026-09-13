"""Verify the exact native subset shown to blind annotators, without predictions."""
from annotations import canonical, document, require
from archive import sha256

FORMAT = 'k1-native-allowlist-v1'
MAX_BYTES = 32 * 1024 * 1024
PAGE_FIELDS = ('number', 'width', 'height', 'start', 'end', 'provenance', 'confidence')
TOKEN_FIELDS = ('start', 'end', 'page', 'rects', 'text', 'provenance')


def native_projection(index_raw, paper_id):
    """Return only the fixed native fields in the frozen first-tranche export."""
    require(len(index_raw) <= MAX_BYTES, 'native export origin exceeds its byte bound')
    index = document(index_raw)['index']
    require(isinstance(index.get('text'), str) and isinstance(index.get('pages'), list)
            and isinstance(index.get('tokens'), list), 'native export lacks text/pages/tokens')
    pages = [{key: row[key] for key in PAGE_FIELDS if key in row} for row in index['pages']]
    tokens = [{key: row[key] for key in TOKEN_FIELDS if key in row} for row in index['tokens']]
    require(1 <= len(pages) <= 400 and len(tokens) <= 500000, 'native export exceeds its inventory bound')
    numbers = [page.get('number') for page in pages]
    require(all(type(number) is int and number > 0 for number in numbers)
            and numbers == sorted(set(numbers)), 'native export page inventory is invalid')
    encoded = index['text'].encode('utf-16-le')
    page_text = []
    for page in pages:
        start, end = page.get('start'), page.get('end')
        require(type(start) is int and type(end) is int and 0 <= start <= end <= len(encoded) // 2,
                'native export page span is invalid')
        try:
            text = encoded[start * 2:end * 2].decode('utf-16-le')
        except UnicodeDecodeError as error:
            raise ValueError('native export page splits a UTF-16 code point') from error
        page_text.append({'page': page['number'], 'text': text})
    # Rectangle fields are native geometry, never arbitrary nested metadata.
    for token in tokens:
        require(isinstance(token.get('rects'), list) and all(isinstance(rect, dict)
                and set(rect) == {'x_min', 'y_min', 'x_max', 'y_max'}
                and all(type(value) in (int, float) for value in rect.values()) for rect in token['rects']),
                'native export token geometry has unsupported fields')
    return {'index_sha256': sha256(index_raw), 'paper_id': paper_id,
            'text': index['text'], 'pages': pages, 'tokens': tokens, 'page_text': page_text,
            'span_unit': 'UTF-16 code units', 'coordinate_system': 'PDF points, top-left origin',
            'exported_fields': ['text', 'pages', 'tokens', 'page_text']}


def verify_native_export(index_raw, export_raw, paper_id, declaration):
    require(len(export_raw) <= MAX_BYTES, 'blind native export exceeds its byte bound')
    require(declaration.get('format') == FORMAT, 'unsupported blind native export format')
    require(declaration.get('index_sha256') == sha256(index_raw), 'blind export names another native index')
    require(type(declaration.get('bytes')) is int and declaration['bytes'] == len(export_raw)
            and declaration.get('sha256') == sha256(export_raw), 'blind export bytes differ from its receipt')
    require(canonical(document(export_raw)) == canonical(native_projection(index_raw, paper_id)),
            'blind export differs from the fixed native allowlist')
    return {'format': FORMAT, 'sha256': sha256(export_raw), 'index_sha256': sha256(index_raw)}
