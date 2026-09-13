'use strict';
// Fixed, trusted Device adapter. PDF text/programs never supply JavaScript.
// Rust validates the full event sequence against the independently captured XML.
var MAX_PIXELS = 4000000, remaining = 6000000, operations = [];
function metadata(image) {
    return {width:image.getWidth(), height:image.getHeight(), components:image.getNumberOfComponents(),
        bits:image.getBitsPerComponent(), image_mask:image.getImageMask(), interpolate:image.getInterpolate(),
        color_key:image.getColorKey(), decode:image.getDecode(), orientation:image.getOrientation()};
}
function pixmapMetadata(pix) {
    var cs = pix.getColorSpace();
    return {width:pix.getWidth(), height:pix.getHeight(), components:pix.getNumberOfComponents(),
        alpha:pix.getAlpha(), bounds:pix.getBounds(), stride:pix.getStride(),
        colorspace_components:cs ? cs.getNumberOfComponents() : null};
}
function boundedImage(meta) {
    var n = meta.width * meta.height;
    if (meta.width <= 0 || meta.height <= 0 || meta.width > 4096 || meta.height > 4096 ||
        n > MAX_PIXELS || meta.components < 1 || meta.components > 4)
        throw new Error('image pixel/component limit');
    return n;
}
function opacity(image) {
    var meta = metadata(image), n = boundedImage(meta);
    if (n > remaining) throw new Error('total mask pixel limit');
    remaining -= n;
    var pix = image.toPixmap(), description = pixmapMetadata(pix);
    if (description.width !== meta.width || description.height !== meta.height ||
        description.components !== 1 || !description.alpha || description.colorspace_components !== null)
        throw new Error('mask is not full-resolution decoded opacity');
    var rows = [];
    for (var y = 0; y < meta.height; ++y) {
        var row = '';
        for (var x = 0; x < meta.width; ++x) {
            var value = pix.getSample(x, y, 0);
            if (value < 0 || value > 255 || Math.floor(value) !== value) throw new Error('invalid opacity');
            row += ('0' + value.toString(16)).slice(-2);
        }
        rows.push(row);
    }
    return {image:meta, pixmap:description, sample_hex_rows:rows};
}
function event(kind, image, matrix, alpha) {
    if (operations.length >= 512) throw new Error('image event limit');
    var row = {event_index:operations.length, kind:kind, matrix:matrix, alpha:alpha, image:metadata(image),
        mask:null, base:null, error:null};
    try {
        if (kind === 'clip_image_mask') row.mask = opacity(image);
        else {
            var mask = image.getMask();
            if (mask) {
                boundedImage(row.image);
                row.base = pixmapMetadata(image.toPixmap());
                row.mask = opacity(mask);
            }
        }
    } catch (error) { row.error = String(error); }
    operations.push(row);
}
if (scriptArgs.length !== 2 || !/^[1-9][0-9]{0,3}$/.test(scriptArgs[1])) throw new Error('invalid page request');
var number = Number(scriptArgs[1]);
var document = Document.openDocument(scriptArgs[0]);
var page = document.loadPage(number - 1);
page.run({
    fillImage:function(image,matrix,alpha) { event('fill_image', image, matrix, alpha); },
    clipImageMask:function(image,matrix) { event('clip_image_mask', image, matrix, null); }
}, Matrix.identity);
print(JSON.stringify({schema_version:1, page:number, bounds:page.getBounds(), operations:operations}));
