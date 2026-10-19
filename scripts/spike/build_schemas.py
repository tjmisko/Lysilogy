"""Generate the paper-artifacts schemas in prompts/.

Strict mode requires every property in `required`, `additionalProperties:
false`, and nulls only through type unions, which is error-prone by hand.
Edit this file and run: python3 -B scripts/spike/build_schemas.py prompts

The core schema carries the fields every extraction needs. Each optional
feature is its own schema so it can be requested on its own, later, without
re-running the core pass.
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

section_kinds = ["abstract", "background", "research_question", "theory", "methods",
                 "data", "results", "discussion", "limitations", "conclusion",
                 "references", "appendix", "other"]
section_families = ["context", "question", "method", "evidence", "interpretation", "caveat", "reference"]

# The fields, gathered per feature so the core schema and each feature schema
# are assembled from the same definitions.
identity = obj(
    title=s("As printed, without line-break hyphens."),
    subtitle=s(null=True),
    authors=arr(obj(name=s("As printed."), affiliation=s(null=True)), "All authors, in printed order."),
    year=i(null=True), venue=s(null=True),
    identifiers=obj(doi=s(null=True), arxiv=s("arXiv identifier without the 'arXiv:' prefix.", null=True)),
    publication_state=enum(["preprint", "published", "unknown"]),
    evidence=evidence_list, confidence=confidence)
paper_type = obj(value=enum(["survey", "method", "empirical", "theory", "dataset", "position", "replication", "resource"]),
         reason=s(), confidence=confidence)
abstract = obj(text=s("The abstract copied verbatim, paragraphs joined with a blank line; null if the paper has none.", null=True),
             page=i(null=True), confidence=confidence)
card_core = obj(
    thesis=obj(text=s("One plain sentence."), evidence=evidence_list, confidence=confidence),
    position=obj(predecessors=arr(position_item, "Work this paper builds on."),
                 targets=arr(position_item, "Work it contrasts with or critiques."),
                 replications=arr(position_item, "Work it replicates or that replicates it.")))
question = obj(text=s("The research question or hypothesis."), evidence=evidence_list, confidence=confidence)
sections = arr(obj(
    title=s("A short topic title, not a breadcrumb of the paper's own headings."),
    kind=enum(section_kinds),
    family=enum(section_families),
    pages=obj(start=i("First 1-based PDF page."), end=i("Last 1-based PDF page.")),
    source_span=obj(start_text=s("A short verbatim phrase from where the unit begins."), start_page=i(),
                    end_text=s("A short verbatim phrase from where the unit ends."), end_page=i()),
    summary=s("One or two sentences that synthesize the whole unit.")),
    "Coherent topic-based reading units, usually 1-2 pages and generally 1-5 pages of content. "
    "Keep related headings, variants, and examples together. Increasing PDF order, non-overlapping spans. Never a list of the paper's own headings.")
references = arr(obj(key=s("The in-text marker, e.g. '12' or 'Smith et al., 2020'."),
                   title=s(null=True), authors=arr(s()), year=i(null=True), venue=s(null=True), doi=s(null=True),
                   page=i()), "Every bibliography entry, in order.")
citations = arr(obj(reference_key=s(), page=i(), context=s("The citing sentence, verbatim.")))
relations = arr(obj(reference_key=s(), relation=enum(relation_kinds), why=s(), quote=s("Verbatim."), page=i(),
                  confidence=confidence))

# Optional features.
supercut = arr(obj(kind=enum(["source", "connector"], "source: the paper's own sentences, verbatim. connector: your short bridging text."),
                 text=s(), page=i("Required for source segments; null for connectors.", null=True)),
             "The argument as ordered segments. Length and quote density follow the paper's importance and prose quality.")
claims = arr(obj(id=s("c1, c2, ..."), statement=s(), support=s("What supports it, in a line."),
               strength=enum(["strong", "moderate", "weak", "speculative"]),
               evidence_refs=arr(s(), "Object labels, numbers, or section names that support it."),
               evidence=evidence_list, confidence=confidence))
method = obj(setting=s(null=True), data=s(null=True), procedure=s(null=True), assumptions=arr(s()),
           evidence=evidence_list, confidence=confidence)
key_numbers = arr(obj(value=s("As printed, with units."), quantity=s(), context=s(),
                    evidence=evidence_list, confidence=confidence))
limitations = arr(obj(text=s(), stated_by=enum(["authors", "model"]), evidence=evidence_list, confidence=confidence))
open_questions = arr(obj(text=s(), stated_by=enum(["authors", "model"]), evidence=evidence_list, confidence=confidence))
objects = arr(obj(kind=enum(["figure", "table", "equation", "algorithm"]),
                label=s("As printed, e.g. 'Figure 3', 'Table 1', '(12)'.", null=True),
                number=s("The number alone, e.g. '3', '1', '12', 'A.2'.", null=True),
                page=i(), caption=s("The printed caption, copied verbatim. Null for equations and for any object without a printed caption; never write one.", null=True),
                latex=s("Equations: LaTeX read from the page image. Null otherwise.", null=True),
                referenced_on_pages=arr(i()), confidence=confidence),
            "Every figure, table, and numbered displayed equation of the requested kinds; also unnumbered displayed equations that matter.")
notation = arr(obj(symbol=s("LaTeX."), meaning=s(), page=i(null=True), evidence=evidence_list, confidence=confidence))
glossary = arr(obj(term=s(), definition=s(), evidence=evidence_list, confidence=confidence))
headline_results = arr(obj(statement=s(), carrier=s("The object label or number that carries the result."),
                         evidence=evidence_list, confidence=confidence))
read_signal = obj(judgment=enum(["read", "skim", "glance", "summary_enough"]),
                  reason=s("Model judgment; say why."))

CORE = {
    "identity": identity,
    "type": paper_type,
    "abstract": abstract,
    "card": card_core,
    "question": question,
    "sections": sections,
    "references": references,
    "citations": citations,
    "relations": relations,
}
FEATURES = {
    "objects": {"objects": objects},
    "supercut": {"supercut": supercut},
    "analysis": {"claims": claims, "method": method, "key_numbers": key_numbers,
                 "limitations": limitations, "open_questions": open_questions,
                 "headline_results": headline_results, "read_signal": read_signal},
    "glossary": {"notation": notation, "glossary": glossary},
}

full = dict(CORE)
for fields in FEATURES.values():
    full.update(fields)
full["card"] = obj(
    thesis=obj(text=s("One plain sentence."), evidence=evidence_list, confidence=confidence),
    position=obj(predecessors=arr(position_item, "Work this paper builds on."),
                 targets=arr(position_item, "Work it contrasts with or critiques."),
                 replications=arr(position_item, "Work it replicates or that replicates it.")),
    headline_results=headline_results, read_signal=read_signal)
paper = obj("The paper-alone artifacts of one scientific paper.", **full)

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

core_schema = obj("The fields every extraction needs: identity, abstract, context, sections, references, citations and relations.", **CORE)
analysis_schema = obj("The paper's claims, method, key numbers, limitations, open questions, headline results and read signal.",
                      **FEATURES["analysis"])
glossary_schema = obj("The symbols and terms a newcomer would need.", **FEATURES["glossary"])
supercut_schema = obj("The argument as ordered segments.", **FEATURES["supercut"])
objects_schema = obj(
    "The figures, tables, algorithms and numbered equations printed on the given pages.",
    objects=json.loads(json.dumps(objects)),
)
objects_schema["properties"]["objects"]["description"] = (
    "Every figure, table and algorithm of the requested kinds whose caption is printed on the given pages, "
    "and every numbered displayed equation of the requested kinds whose number is printed on them. "
    "One object per printed label; never a range."
)

for name, schema in [("paper-artifacts", paper), ("paper-core", core_schema),
                     ("paper-objects", objects_schema), ("paper-supercut", supercut_schema),
                     ("paper-analysis", analysis_schema), ("paper-glossary", glossary_schema),
                     ("paper-artifacts-check", check), ("paper-artifacts-repair", repair)]:
    with open(f"{out}/{name}.schema.json", "w") as f:
        json.dump(schema, f, indent=2)
        f.write("\n")
