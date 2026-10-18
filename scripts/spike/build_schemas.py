"""Generate the paper-artifacts schemas in prompts/.

Strict mode requires every property in `required`, `additionalProperties:
false`, and nulls only through type unions, which is error-prone by hand.
Edit this file and run: python3 -B scripts/spike/build_schemas.py prompts
"""
import json
import sys

out = sys.argv[1] if len(sys.argv) > 1 else "prompts"

def obj(desc=None, **props):
    o = {"type": "object", "additionalProperties": False, "required": list(props), "properties": props}
    if desc: o["description"] = desc
    return o
def s(desc=None, null=False):
    o = {"type": ["string", "null"] if null else "string"}
    if desc: o["description"] = desc
    return o
def i(desc=None, null=False):
    o = {"type": ["integer", "null"] if null else "integer"}
    if desc: o["description"] = desc
    return o
def arr(items, desc=None):
    o = {"type": "array", "items": items}
    if desc: o["description"] = desc
    return o
def enum(values, desc=None):
    o = {"type": "string", "enum": values}
    if desc: o["description"] = desc
    return o

evidence = obj("A verbatim passage from the paper and the page it is on.",
    quote=s("Copied exactly from the page text, 8 to 300 characters."),
    page=i("1-based page number."))
evidence_list = arr(evidence, "Verbatim support. Empty only when the item is a model judgment.")
confidence = enum(["high", "medium", "low"], "How sure you are that this item is correct.")
position_item = obj(
    reference_key=s("The key of the entry in `references`, or null if it is not in the bibliography.", null=True),
    cited_work=s("Short name of the cited work, e.g. 'Smith et al. 2020'."),
    why=s("One plain-language line on the relation."),
    evidence=evidence_list, confidence=confidence)
relation_kinds = ["background", "uses_method", "uses_data", "extends", "supports", "contrasts", "critiques", "compares", "other"]

paper = obj(
    "The paper-alone artifacts of one scientific paper.",
    identity=obj(
        title=s("As printed, without line-break hyphens."),
        subtitle=s(null=True),
        authors=arr(obj(name=s("As printed."), affiliation=s(null=True)), "All authors, in printed order."),
        year=i(null=True), venue=s(null=True),
        identifiers=obj(doi=s(null=True), arxiv=s("arXiv identifier without the 'arXiv:' prefix.", null=True)),
        publication_state=enum(["preprint", "published", "unknown"]),
        evidence=evidence_list, confidence=confidence),
    type=obj(value=enum(["survey", "method", "empirical", "theory", "dataset", "position", "replication", "resource"]),
             reason=s(), confidence=confidence),
    abstract=obj(text=s("The abstract copied verbatim, paragraphs joined with a blank line; null if the paper has none.", null=True),
                 page=i(null=True), confidence=confidence),
    card=obj(
        thesis=obj(text=s("One plain sentence."), evidence=evidence_list, confidence=confidence),
        position=obj(predecessors=arr(position_item, "Work this paper builds on."),
                     targets=arr(position_item, "Work it contrasts with or critiques."),
                     replications=arr(position_item, "Work it replicates or that replicates it.")),
        headline_results=arr(obj(statement=s(), carrier=s("The object label or number that carries the result."),
                                 evidence=evidence_list, confidence=confidence)),
        read_signal=obj(judgment=enum(["read", "skim", "glance", "summary_enough"]),
                        reason=s("Model judgment; say why."))),
    supercut=arr(obj(kind=enum(["source", "connector"], "source: the paper's own sentences, verbatim. connector: your short bridging text."),
                     text=s(), page=i("Required for source segments; null for connectors.", null=True)),
                 "The argument as ordered segments. Length and quote density follow the paper's importance and prose quality."),
    question=obj(text=s("The research question or hypothesis."), evidence=evidence_list, confidence=confidence),
    claims=arr(obj(id=s("c1, c2, ..."), statement=s(), support=s("What supports it, in a line."),
                   strength=enum(["strong", "moderate", "weak", "speculative"]),
                   evidence_refs=arr(s(), "Object labels, numbers, or section names that support it."),
                   evidence=evidence_list, confidence=confidence)),
    method=obj(setting=s(null=True), data=s(null=True), procedure=s(null=True), assumptions=arr(s()),
               evidence=evidence_list, confidence=confidence),
    key_numbers=arr(obj(value=s("As printed, with units."), quantity=s(), context=s(),
                        evidence=evidence_list, confidence=confidence)),
    limitations=arr(obj(text=s(), stated_by=enum(["authors", "model"]), evidence=evidence_list, confidence=confidence)),
    open_questions=arr(obj(text=s(), stated_by=enum(["authors", "model"]), evidence=evidence_list, confidence=confidence)),
    objects=arr(obj(kind=enum(["figure", "table", "equation", "algorithm"]),
                    label=s("As printed, e.g. 'Figure 3', 'Table 1', '(12)'.", null=True),
                    number=s("The number alone, e.g. '3', '1', '12', 'A.2'.", null=True),
                    page=i(), caption=s("The printed caption, copied verbatim. Null for equations and for any object without a printed caption; never write one.", null=True),
                    latex=s("Equations: LaTeX read from the page image. Null otherwise.", null=True),
                    plain_language=s(), why_it_matters=s(),
                    referenced_on_pages=arr(i()), confidence=confidence),
                "Every figure, table, and numbered displayed equation; also unnumbered displayed equations that matter."),
    notation=arr(obj(symbol=s("LaTeX."), meaning=s(), page=i(null=True), evidence=evidence_list, confidence=confidence)),
    glossary=arr(obj(term=s(), definition=s(), evidence=evidence_list, confidence=confidence)),
    references=arr(obj(key=s("The in-text marker, e.g. '12' or 'Smith et al., 2020'."),
                       title=s(null=True), authors=arr(s()), year=i(null=True), venue=s(null=True), doi=s(null=True),
                       page=i()), "Every bibliography entry, in order."),
    citations=arr(obj(reference_key=s(), page=i(), context=s("The citing sentence, verbatim."))),
    relations=arr(obj(reference_key=s(), relation=enum(relation_kinds), why=s(), quote=s("Verbatim."), page=i(),
                      confidence=confidence)),
)


check = obj(
    "Corrections to a draft of paper-artifacts. Only correct or flag; every correction needs verbatim evidence.",
    corrections=arr(obj(
        path=s("JSON Pointer into the draft, e.g. '/identity/title' or '/objects/3/latex'. For add, point at the array, e.g. '/objects'."),
        action=enum(["replace", "remove", "add", "flag"]),
        value_json=s("The new value encoded as a JSON string, for replace and add; null for remove and flag.", null=True),
        reason=s(), evidence=evidence_list)))

repair = obj(
    "Repairs for quotes that are not verbatim in the paper.",
    repairs=arr(obj(id=s("The id given for the failing quote."),
                    action=enum(["replace_quote", "withdraw"]),
                    quote=s("The exact text from the page, for replace_quote.", null=True),
                    page=i(null=True))))

# Chunked mode: pass 1 fills everything but objects; a separate pass reads a
# few pages at a time and returns only the objects printed on them.
core = json.loads(json.dumps(paper))
del core["properties"]["objects"]
core["required"].remove("objects")
core["description"] = "The paper-alone artifacts of one scientific paper, without the object inventory."
chunk_objects = obj(
    "The figures, tables, algorithms and numbered equations printed on the given pages.",
    objects=json.loads(json.dumps(paper["properties"]["objects"])),
)
chunk_objects["properties"]["objects"]["description"] = (
    "Every figure, table and algorithm whose caption is printed on the given pages, and every numbered "
    "displayed equation whose number is printed on them. One object per printed label; never a range."
)

for name, schema in [("paper-artifacts", paper), ("paper-artifacts-core", core), ("paper-objects", chunk_objects),
                     ("paper-artifacts-check", check), ("paper-artifacts-repair", repair)]:
    with open(f"{out}/{name}.schema.json", "w") as f:
        json.dump(schema, f, indent=2)
        f.write("\n")
