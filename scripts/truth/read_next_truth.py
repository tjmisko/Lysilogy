"""K7 fold mechanics. Actual truth publication waits for verified scale/local mapping.

Inputs are derived citation observations, never arbitrary provider JSON or cached
graph features. Every edge is oriented citing -> cited, including incoming views.
Candidate nodes must come from independently mapped scale/local papers; a node
discovered only in the held-out bibliography cannot leak into the candidate set.
"""
import collections
import re

from reference_truth import TruthError, doi, fingerprint, text, year


def alias_key(value):
    if (not isinstance(value, str) or not value or len(value) > 512
            or any(c.isspace() or ord(c) < 32 or ord(c) == 127 for c in value)):
        raise TruthError("Graph identity must be a bounded nonempty identifier")
    if normalized := doi(value):
        return "doi:" + normalized
    if value.startswith("arxiv:"):
        identifier = value[len("arxiv:"):]
        if not re.fullmatch(r"[0-9]{4}\.[0-9]{4,5}(v[0-9]+)?", identifier):
            raise TruthError("Invalid arXiv graph identity")
        # Withhold every version's duplicate outgoing edges for the held-out paper.
        return "arxiv:" + re.sub(r"v[0-9]+$", "", identifier)
    return value


def normalize_graph(observed):
    """Canonicalize explicitly supplied identities; do not infer aliases from names/titles."""
    if set(observed) != {"nodes", "views"}:
        raise TruthError("Graph observations may contain only nodes and oriented edge views")
    nodes, aliases = {}, {}
    for node in observed["nodes"]:
        if set(node) - {"id", "aliases", "title", "year", "cohorts"}:
            raise TruthError("Graph nodes cannot carry raw provider records or precomputed features")
        identifier = node["id"]
        alias_key(identifier)
        if identifier in nodes:
            raise TruthError("Duplicate canonical graph node")
        cohorts = node["cohorts"]
        if not isinstance(cohorts, list) or not cohorts or not set(cohorts) <= {"scale", "local"}:
            raise TruthError("K7 candidates must be independently mapped scale/local papers")
        raw_aliases = node.get("aliases", [])
        if not isinstance(raw_aliases, list):
            raise TruthError("Graph aliases must be a list")
        keys = sorted({alias_key(value) for value in [identifier, *raw_aliases]})
        for key in keys:
            if key in aliases and aliases[key] != identifier:
                raise TruthError("Ambiguous graph alias belongs to different candidate papers")
            aliases[key] = identifier
        nodes[identifier] = {"id": identifier, "aliases": keys, "title": text(node.get("title")),
                             "year": year(node.get("year")), "cohorts": sorted(set(cohorts))}
    views, coverage = {}, {}
    for view in observed["views"]:
        if set(view) != {"name", "edges"} or not text(view["name"]) or view["name"] in views:
            raise TruthError("Graph views need unique names and oriented edges only")
        edges, counts = set(), collections.Counter()
        for edge in view["edges"]:
            if set(edge) != {"source", "target", "source_record"} or not text(edge["source_record"]):
                raise TruthError("Graph edges require citing source, cited target and provenance")
            counts["observed"] += 1
            source = aliases.get(alias_key(edge["source"]))
            target = aliases.get(alias_key(edge["target"]))
            if source is None or target is None:
                counts["outside_candidate_universe"] += 1
                continue
            row = (source, target, edge["source_record"])
            counts["duplicate_evidence" if row in edges else "retained"] += 1
            edges.add(row)
        views[view["name"]] = [{"source": source, "target": target, "source_record": record}
                               for source, target, record in sorted(edges)]
        coverage[view["name"]] = dict(sorted(counts.items()))
    return {"schema_version": 1, "nodes": [nodes[key] for key in sorted(nodes)],
            "views": {name: views[name] for name in sorted(views)}, "coverage": coverage}


def canonical_query(graph, query):
    key = alias_key(query)
    matches = [node["id"] for node in graph["nodes"] if key in node["aliases"]]
    if len(matches) != 1:
        raise TruthError("Fold query is not one unambiguous mapped candidate")
    return matches[0]


def fold_features(graph, query):
    """Withhold the query's outgoing edges in ALL views before deriving any graph features."""
    held_out = canonical_query(graph, query)
    nodes = [{key: node[key] for key in ("id", "title", "year")} for node in graph["nodes"]]
    # Evidence locations can identify a whole provider record whose raw payload
    # also contains held-out references. They belong in truth provenance only.
    views = {name: [{key: edge[key] for key in ("source", "target")}
                    for edge in edges if edge["source"] != held_out]
             for name, edges in graph["views"].items()}
    union = {(edge["source"], edge["target"]) for edges in views.values() for edge in edges}
    outgoing = {node["id"]: [] for node in nodes}
    incoming = {node["id"]: [] for node in nodes}
    for source, target in sorted(union):
        outgoing[source].append(target)
        incoming[target].append(source)
    # In particular, no raw reference list, cached degree, or unfiltered provider
    # payload is forwarded. These projections all come from the filtered union.
    return {"query": held_out, "nodes": nodes, "views": views,
            "outgoing": outgoing, "incoming": incoming,
            "degrees": {node["id"]: {"out": len(outgoing[node["id"]]), "in": len(incoming[node["id"]])}
                        for node in nodes}}


def fold_plan(graph, seed="read-next-truth-v1", label_view="bibliography"):
    """Return compact fold specifications; the base graph is retained exactly once."""
    if label_view not in graph["views"]:
        raise TruthError("K7 needs the actual bibliography label view")
    positives = collections.defaultdict(set)
    self_edges = 0
    for edge in graph["views"][label_view]:
        if edge["source"] == edge["target"]:
            self_edges += 1
        else:
            positives[edge["source"]].add(edge["target"])
    graph_hash = fingerprint(graph)
    ordered = sorted(positives, key=lambda identifier: (fingerprint([seed, identifier]), identifier))
    folds = [{"fold_id": "K7-" + fingerprint([seed, identifier])[:24], "query": identifier,
              "expected_targets": sorted(positives[identifier]), "graph_sha256": graph_hash}
             for identifier in ordered]
    return {"schema_version": 1, "seed": seed, "graph_sha256": graph_hash,
            "label_view": label_view, "folds": folds,
            "coverage": {"candidate_papers": len(graph["nodes"]), "eligible_folds": len(folds),
                         "papers_without_in_universe_references": len(graph["nodes"]) - len(folds),
                         "self_reference_evidence_excluded": self_edges,
                         "observed_edges": graph["coverage"]}}


def features_for_fold(graph, fold):
    if fold.get("graph_sha256") != fingerprint(graph):
        raise TruthError("K7 fold graph differs from its frozen fingerprint")
    return fold_features(graph, fold["query"])
