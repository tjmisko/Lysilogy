"""Fixed O11 panel arithmetic; no detector, model, or network calls."""


def score_panel(prediction, panelists, valid_ids):
    """Score three ranked positions against three independent top-three votes.

    The caller binds the frozen paper, source IDs, prompt and independently
    reviewed vote receipts. Order in prediction is rank order; an invalid or
    duplicate top-three slot is never backfilled by a later ranked position.
    """
    if not isinstance(prediction, list) or any(not isinstance(value, str) for value in prediction):
        raise ValueError("model ranking must be an unambiguous ordered list of IDs")
    if len(prediction) > 5:
        raise ValueError("model ranking exceeds the declared five-position output contract")
    if not isinstance(panelists, list) or len(panelists) != 3:
        raise ValueError("O11 requires exactly three independent panelists")
    valid_ids = set(valid_ids)
    for vote in panelists:
        if not isinstance(vote, list) or len(vote) != 3 or any(not isinstance(value, str) for value in vote) or len(set(vote)) != 3 or not set(vote) <= valid_ids:
            raise ValueError("each panel vote requires three distinct valid source IDs")
    top_three = set(prediction[:3]) & valid_ids
    overlaps = [len(top_three & set(vote)) for vote in panelists]
    return {"numerator": sum(overlaps), "denominator": 9,
            "agreement": sum(overlaps) / 9, "panel_overlaps": overlaps,
            "valid_unique_top_three": len(top_three), "missing_top_three_slots": 3 - len(top_three),
            "ignored_later_positions": max(0, len(prediction) - 3)}


def score_frozen_panels(papers, predictions):
    """Every frozen paper contributes nine opportunities, including failures."""
    if not papers or len({paper["arxiv_id"] for paper in papers}) != len(papers):
        raise ValueError("the frozen panel cohort must contain unique papers")
    if not isinstance(predictions, dict) or not set(predictions) <= {paper["arxiv_id"] for paper in papers}:
        raise ValueError("predictions contain papers outside the frozen panel cohort")
    results = {paper["arxiv_id"]: score_panel(predictions.get(paper["arxiv_id"], []), paper["panelists"], paper["valid_ids"]) for paper in papers}
    numerator = sum(row["numerator"] for row in results.values())
    denominator = 9 * len(papers)
    return {"numerator": numerator, "denominator": denominator, "agreement": numerator / denominator, "papers": results}
