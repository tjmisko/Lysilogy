"""Validate independent manual figure/table evidence without changing automatic truth."""
from copy import deepcopy
import json
import math

from align import TextAlignment
from archive import sha256


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False, allow_nan=False).encode()


def require(condition, message):
    if not condition:
        raise ValueError(message)


def document(raw):
    require(isinstance(raw, bytes) and len(raw) <= 32 * 1024 * 1024, "annotation evidence exceeds its byte bound")
    def pairs(items):
        output = {}
        for key, value in items:
            require(key not in output, "annotation JSON repeats a field")
            output[key] = value
        return output
    def invalid(value):
        raise ValueError("annotation JSON has a non-finite value: " + value)
    result = json.loads(raw, object_pairs_hook=pairs, parse_constant=invalid)
    require(isinstance(result, dict), "annotation evidence must be a JSON object")
    return result


def apply_regions(candidate_raw, index_raw, annotation_raw, region_review_raw, association_raw, association_review_raw):
    """Attach an independently reviewed manual overlay, retaining automatic exclusions.

    Inputs are exact frozen receipt bytes, not booleans asserting that a review
    happened. Real artifact hashes and rendered-page receipts are verified by the
    offline builder before this pure validation/assembly step.
    """
    candidate, index = document(candidate_raw), document(index_raw)["index"]
    annotation, region_review = document(annotation_raw), document(region_review_raw)
    association, association_review = document(association_raw), document(association_review_raw)
    require(all(row.get("schema_version") == 1 for row in (annotation, region_review, association, association_review)), "unsupported manual evidence schema")
    hashes = {"source_candidate_file_sha256": sha256(candidate_raw), "source_inventory_sha256": candidate["source_inventory_sha256"],
              "region_annotation_sha256": sha256(annotation_raw), "independent_region_review_sha256": sha256(region_review_raw),
              "source_sha256": candidate["source_sha256"], "pdf_sha256": candidate["pdf_sha256"]}
    require(candidate["index"]["sha256"] == sha256(index_raw), "annotation index bytes disagree with frozen candidate")
    require(sha256(canonical(candidate["source_inventory"])) == candidate["source_inventory_sha256"], "source inventory hash mismatch")
    require(all(association.get(key) == value for key, value in hashes.items()), "source-association provenance differs from exact artifacts")
    require(association["index"] == candidate["index"], "source-association index differs from candidate")
    require(region_review.get("verdict") == "clear_for_independent_region_candidate" and region_review.get("findings") == [], "full-region evidence has no clear independent review")
    require(region_review.get("annotation_sha256") == sha256(annotation_raw), "region review binds different annotation bytes")
    require(association_review.get("verdict") == "clear_source_to_visual_associations" and association_review.get("findings") == [], "source associations have no clear independent review")
    require(association_review.get("reviewed_association_sha256") == sha256(association_raw), "association review binds different bytes")
    require(all(association_review.get("verified_hashes", {}).get(key) == value for key, value in hashes.items()), "association reviewer verified different artifacts")
    require(association_review.get("index_sha256_verified") == candidate["index"]["sha256"], "association reviewer verified another index")
    require(annotation.get("annotator") and association.get("annotator"), "annotation authorship is absent")
    require(region_review.get("reviewer") and region_review["reviewer"] != annotation["annotator"], "region review is not independent of its annotator")
    require(association_review.get("reviewer") and association_review["reviewer"] != association["annotator"], "association review is not independent of its annotator")
    require(all(row.get("arxiv_id") == candidate["arxiv_id"] and row.get("pdf_sha256", candidate["pdf_sha256"]) == candidate["pdf_sha256"] for row in (annotation, region_review, association, association_review)), "annotation paper identity mismatch")
    coverage = candidate["source_inventory"]["coverage"]
    require(not coverage.get("unsupported_source_semantics") and not coverage.get("unsupported_object_environments"), "manual captions cannot repair unknown source inventories")
    require(not candidate.get("duplicate_span_claims"), "manual evidence cannot hide duplicate source span claims")
    pages = {page["number"]: page for page in index["pages"]}
    require(len(pages) == len(index["pages"]), "index repeats a page number")
    page_numbers = sorted(pages)
    coordinates = annotation.get("coordinate_system", {})
    require(coordinates.get("page_numbers") == "one-based" and coordinates.get("origin") == "top-left of unrotated PDF CropBox" and coordinates.get("unit") == "PDF point", "unsupported manual coordinate system")
    require(all(page["width"] == coordinates.get("page_width") and page["height"] == coordinates.get("page_height") for page in pages.values()), "manual page geometry differs from the index")
    require(set(annotation.get("page_render_sha256", {})) == {f"page-{number}.png" for number in page_numbers}, "manual evidence lacks a render for every page")
    require(annotation.get("pages_inspected") == page_numbers and region_review.get("pages_visually_inspected") == page_numbers and association.get("pages_visually_inspected") == page_numbers, "manual evidence must inspect the whole paper")
    objects = {row["id"]: row for row in candidate["source_inventory"]["objects"] if row["kind"] in ("figure", "table")}
    require(len(objects) == sum(row["kind"] in ("figure", "table") for row in candidate["source_inventory"]["objects"]), "source object identity is duplicated")
    require(objects, "manual evidence has no figure/table source inventory")
    expected_counts = {"figures": sum(row["kind"] == "figure" for row in objects.values()), "tables": sum(row["kind"] == "table" for row in objects.values())}
    require(annotation.get("complete_printed_inventory") == expected_counts and association.get("complete_figure_table_inventory") == expected_counts, "manual inventory counts disagree with source")
    require(all(region_review.get("inventory", {}).get(key) == value and association_review.get("complete_inventory", {}).get(key) == value for key, value in expected_counts.items()), "reviewed inventory counts disagree with source")
    require(region_review.get("inventory", {}).get("complete") is True, "region inventory was not reviewed as complete")
    mapped = association["associations"]
    require(len(mapped) == len(objects) and {row["source_object_id"] for row in mapped} == set(objects), "source associations are incomplete or duplicated")
    reviewed = {row["source_object_id"]: row for row in association_review["associations"]}
    require(len(association_review["associations"]) == len(objects) and set(reviewed) == set(objects), "independent association review has an incomplete inventory")
    regions = {(row["kind"], row["printed_label"], row["page"]): row for row in annotation["regions"]}
    require(len(annotation["regions"]) == len(objects) and len(regions) == len(objects), "region inventory is incomplete or duplicated")
    require(len({(row["kind"], row["printed_label"]) for row in annotation["regions"]}) == len(objects), "printed object identity is duplicated")
    require({(row["kind"], row["printed_label"], row["page"]) for row in mapped} == set(regions), "source and region identities do not match one-to-one")
    aligner = TextAlignment(index)
    verified_objects = []
    for binding in mapped:
        source = objects[binding["source_object_id"]]
        review = reviewed[source["id"]]
        require(isinstance(source["caption"], str) and source["caption"] and sha256(source["caption"].encode()) == source["text_sha256"], "source caption text differs from its retained hash")
        require(source["kind"] == binding["kind"] and source["text_sha256"] == binding["source_caption_sha256"], "association names another source caption")
        require(all(review.get(key) == binding.get(key) for key in ("kind", "printed_label", "page", "source_caption_sha256")) and review.get("review_verdict") == "confirmed" and review.get("source_members") == source["source_members"], "independent source association differs from proposal")
        unknown = source.get("unsupported_commands", {})
        if unknown:
            require(unknown == {"unverified_script_binding": 1}, "manual figure overlay cannot repair other unsupported rendering")
            manual = association_review.get("manual_math_caption_review", {})
            require(manual.get("source_object_id") == source["id"] and manual.get("verdict") == "confirmed_separately_from_automatic_text_alignment" and manual.get("automatic_script_binding_withholding_retained") is True and manual.get("automatic_confidence_changed") is False, "script-containing caption lacks separate reviewed visual evidence")
            require(manual.get("page") == binding["page"] and manual.get("printed_label") == binding["printed_label"] and manual.get("source_member") in source["source_members"], "manual math review names a different source object")
        # Visual/source review establishes the full caption and any script
        # binding. Text folding here only locates that already verified caption.
        span, reason = aligner.unique(source["caption"])
        require(span is not None, "reviewed caption lacks a unique index location: " + str(reason))
        page = pages.get(binding["page"])
        require(page is not None and page["start"] <= span["start"] < span["end"] <= page["end"], "reviewed caption span is not on its named page")
        region = regions[(binding["kind"], binding["printed_label"], binding["page"])]["pdf_points"]
        values = [region[key] for key in ("x", "y", "width", "height")]
        require(all(type(value) in (int, float) and math.isfinite(value) for value in values), "region rectangle is non-finite")
        x, y, width, height = values
        require(x >= 0 and y >= 0 and width > 0 and height > 0 and x + width <= page["width"] and y + height <= page["height"], "region rectangle falls outside its PDF page")
        verified_objects.append({"id": source["id"], "kind": source["kind"], "labels": source["labels"], "printed_label": binding["printed_label"],
                                 "spans": [{"start": span["start"], "end": span["end"]}], "source_members": source["source_members"], "text_sha256": source["text_sha256"],
                                 "region": [{"page": binding["page"], "rect": {"x_min": x, "y_min": y, "x_max": x + width, "y_max": y + height}}],
                                 "alignment_method": "independently reviewed source-to-visual association with separate full-region annotation",
                                 "automatic_alignment_changed": False})
    require(len({(row["spans"][0]["start"], row["spans"][0]["end"]) for row in verified_objects}) == len(objects), "manual captions claim the same printed span")
    output = deepcopy(candidate)
    output["manual_figure_table_overlay"] = {"objects": verified_objects, "metric_eligibility": {"O1": True, "O2": True},
                                            "evidence_hashes": {**hashes, "source_association_sha256": sha256(association_raw), "source_association_review_sha256": sha256(association_review_raw)},
                                            "automatic_candidate_retained": True, "final_k1_publication": False}
    return output
