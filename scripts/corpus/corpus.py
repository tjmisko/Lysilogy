#!/usr/bin/env python3
"""Build K0 outside the repository, using only Python's standard library.

Network is used only by explicit corpus build commands. See README.md for
policy, provenance, frozen selection semantics, and the isolated mapping command.
"""

import argparse
import base64
import contextlib
import datetime as dt
import email.utils
import fcntl
import hashlib
import http.client
import json
import math
import os
from pathlib import Path
import re
import shutil
import sqlite3
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
import xml.etree.ElementTree as ET

OAI = "https://oaipmh.arxiv.org/oai"
GCS = "https://storage.googleapis.com"
EXPORT = "https://export.arxiv.org/e-print/"
USER_AGENT = "Lysilogy-K0/1.0 (research corpus; https://github.com/tjmisko/Lysilogy)"
PROXY_ENV_VARS = ("HTTPS_PROXY", "https_proxy")
MIB = 1024 * 1024
GIB = 1024 * MIB
NS = {"o": "http://www.openarchives.org/OAI/2.0/", "a": "http://arxiv.org/OAI/arXiv/"}
ID_PATTERN = re.compile(r"\d{4}\.\d{4,5}")
OBJECT_PATTERN = re.compile(r"arxiv/arxiv/pdf/(\d{4})/(\d{4}\.\d{4,5})v([1-9]\d*)\.pdf")


class CorpusError(Exception):
    """Expected, actionable corpus build failure."""


def now():
    return dt.datetime.now(dt.timezone.utc).isoformat()


def canonical(value):
    return json.dumps(value, sort_keys=True, ensure_ascii=False, separators=(",", ":"))


def fingerprint(value):
    return hashlib.sha256(canonical(value).encode()).hexdigest()


def fingerprint_records(records):
    """Hash the canonical array without allocating another full metadata copy."""
    digest = hashlib.sha256(b"[")
    for index, record in enumerate(records):
        if index:
            digest.update(b",")
        digest.update(canonical(record).encode())
    digest.update(b"]")
    return digest.hexdigest()


def sync_directory(path):
    descriptor = os.open(path, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def durable_directory(path):
    if path.is_symlink():
        raise CorpusError("Corpus storage cannot use symlinks")
    if not path.exists():
        durable_directory(path.parent)
        path.mkdir()
        sync_directory(path.parent)


def atomic_json(path, value):
    atomic_text(path, canonical(value) + "\n")


def atomic_text(path, value):
    durable_directory(path.parent)
    partial = path.with_name(path.name + ".partial")
    try:
        partial.unlink(missing_ok=True)
        with partial.open("x", encoding="utf-8") as out:
            out.write(value)
            out.flush()
            os.fsync(out.fileno())
        partial.replace(path)
        sync_directory(path.parent)
    finally:
        partial.unlink(missing_ok=True)


def immutable_text(path, value):
    """Publish durable archive bytes without ever replacing an existing archive."""
    durable_directory(path.parent)
    if path.parent.is_symlink() or path.is_symlink():
        raise CorpusError("Corpus archive cannot use symlinks")
    if path.exists():
        if path.read_bytes() != value.encode():
            raise CorpusError("Existing corpus archive differs; leaving it untouched")
        return
    partial = path.with_name(path.name + ".partial")
    try:
        partial.unlink(missing_ok=True)
        with partial.open("x", encoding="utf-8") as out:
            out.write(value)
            out.flush()
            os.fsync(out.fileno())
        os.link(partial, path)
        sync_directory(path.parent)
        sync_directory(path.parent.parent)
    finally:
        partial.unlink(missing_ok=True)


def read_json(path, default=None):
    if not path.exists():
        return default
    with path.open(encoding="utf-8") as source:
        return json.load(source)


def validate_root(raw):
    root = Path(raw).expanduser()
    # Reject these lexically before resolving symlinks or touching the filesystem.
    if any(part in {".lysilogy", "local-articles", ".env", ".secrets"} or part.startswith(".env.")
           for part in root.parts):
        raise CorpusError("Corpus root must be separate from the library and data root")
    root = root.resolve()
    repository = Path(__file__).resolve().parents[2]
    if ".worktrees" in repository.parts:
        repository = Path(*repository.parts[:repository.parts.index(".worktrees")])
    if root == repository or root.is_relative_to(repository) or repository.is_relative_to(root):
        raise CorpusError("Corpus root must be outside the repository")
    if root == Path("/tmp") or root.is_relative_to("/tmp") or ".lysilogy" in root.parts or "local-articles" in root.parts:
        raise CorpusError("Corpus root cannot be /tmp, a library, or a data root")
    return root


def check_space(root, projected, floor, disk_usage=shutil.disk_usage):
    free = disk_usage(root).free
    result = {"free_bytes": free, "projected_additional_bytes": projected, "floor_bytes": floor}
    if free - projected < floor:
        raise CorpusError(f"Insufficient disk space: {canonical(result)}")
    return result


class Pacer:
    """One request every three seconds, including retries; no burst concurrency."""

    def __init__(self, interval=3.0, clock=time.monotonic, sleep=time.sleep):
        self.interval = max(3.0, interval)
        self.clock = clock
        self.sleep = sleep
        self.next_at = 0.0

    def wait(self):
        delay = max(0.0, self.next_at - self.clock())
        if delay:
            self.sleep(delay)
        self.next_at = self.clock() + self.interval


def retry_after(value, timestamp=None):
    try:
        return max(0.0, float(value))
    except (ValueError, TypeError):
        try:
            parsed = email.utils.parsedate_to_datetime(value).timestamp()
            return max(0.0, parsed - (time.time() if timestamp is None else timestamp))
        except (ValueError, TypeError, OverflowError):
            return 0.0


def validate_url(url):
    parsed = urllib.parse.urlsplit(url)
    if (parsed.scheme != "https" or parsed.hostname not in
            {"oaipmh.arxiv.org", "storage.googleapis.com", "export.arxiv.org"}
            or parsed.username or parsed.password or parsed.port not in (None, 443)):
        raise CorpusError("Corpus request or redirect left the approved HTTPS hosts")


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        # A changed endpoint needs deliberate review; never forward untrusted URLs.
        raise CorpusError(f"Unexpected HTTP redirect ({code}); check the documented endpoint")


def persist_rate_deadline(rate_file, deadline):
    """Durably retain server cooldowns before control can sleep or return."""
    rate_file.seek(0)
    previous = float(rate_file.read().strip() or 0)
    rate_file.seek(0)
    rate_file.truncate()
    rate_file.write(str(max(previous, deadline)))
    rate_file.flush()
    os.fsync(rate_file.fileno())


def configured_proxy(variable):
    """Read only an explicitly selected proxy; never include its value in errors."""
    if variable is None:
        return {}
    if variable not in PROXY_ENV_VARS:
        raise CorpusError("Proxy environment variable must be HTTPS_PROXY or https_proxy")
    value = os.environ.get(variable, "")
    if not value:
        raise CorpusError("The selected HTTPS proxy environment variable is missing or empty")
    try:
        parsed = urllib.parse.urlsplit(value)
        if parsed.scheme == "https":
            raise CorpusError("HTTPS-scheme proxies are unsupported by this transport; configure an approved HTTP CONNECT proxy")
        valid = (parsed.scheme == "http" and parsed.hostname
                 and parsed.port != 0 and parsed.path in {"", "/"}
                 and not parsed.query and not parsed.fragment
                 and not any(ord(char) <= 32 or ord(char) == 127 for char in value))
    except ValueError:
        valid = False
    if not valid:
        raise CorpusError("The selected HTTPS proxy must be a valid http:// proxy URL without a path, query or fragment")
    return {"https": value}


class Http:
    def __init__(self, cache_root=None, sleep=time.sleep, clock=time.time, proxy_env=None):
        self.clock = clock
        self.cache_root = cache_root or Path.home() / ".cache/lysilogy"
        self.sleep = sleep
        proxies = configured_proxy(proxy_env)
        self.proxy_enabled = bool(proxies)
        self.opener = urllib.request.build_opener(urllib.request.ProxyHandler(proxies), NoRedirect())

    @staticmethod
    def proxy_failure(error):
        # urllib errors can echo an authenticated proxy URL. Report only status/type
        # and suppress exception chaining at the call site, never transport secrets.
        detail = f"HTTP {error.code}" if isinstance(error, urllib.error.HTTPError) else type(error).__name__
        return CorpusError(f"Configured HTTPS proxy request failed ({detail}); check proxy access and corpus host permissions")

    @contextlib.contextmanager
    def open(self, url):
        validate_url(url)
        if not self.proxy_enabled:
            with self._open(url) as response:
                yield response
            return
        try:
            with self._open(url) as response:
                yield response
        except (KeyboardInterrupt, SystemExit, GeneratorExit, CorpusError) as error:
            # Keep cancellation and known corpus failures intact, but suppress a
            # transport exception left as context during persistence or cleanup.
            raise error from None
        except Exception as error:
            # The response's read/close and retry handler are part of the same
            # boundary as connection setup; none may echo proxy credentials.
            raise self.proxy_failure(error) from None

    @contextlib.contextmanager
    def _open(self, url):
        arxiv = urllib.parse.urlsplit(url).hostname != "storage.googleapis.com"
        # This lock is shared across corpus roots/processes. Hold it until the
        # response closes: the policy also limits simultaneous connections.
        with contextlib.ExitStack() as stack:
            rate_file = None
            if arxiv:
                self.cache_root.mkdir(parents=True, exist_ok=True)
                rate_file = stack.enter_context((self.cache_root / "arxiv-http.lock").open("a+"))
                fcntl.flock(rate_file, fcntl.LOCK_EX)
            for attempt in range(6):
                if rate_file:
                    rate_file.seek(0)
                    raw = rate_file.read().strip()
                    pacer = Pacer(clock=self.clock, sleep=self.sleep)
                    pacer.next_at = float(raw or 0)
                    pacer.wait()
                    persist_rate_deadline(rate_file, pacer.next_at)
                try:
                    response = self.opener.open(urllib.request.Request(url, headers={"User-Agent": USER_AGENT}), timeout=90)
                    break
                except urllib.error.HTTPError as error:
                    try:
                        delay = max(retry_after(error.headers.get("Retry-After"), timestamp=self.clock()),
                                    3.0 * 2**attempt)
                        if rate_file:
                            # Save even on the final retry, before raising or an
                            # interruptible sleep releases the shared connection lock.
                            persist_rate_deadline(rate_file, self.clock() + delay)
                    finally:
                        # Cancellation during persistence must still close the
                        # response, including its deferred resource-warning state.
                        error.close()
                    if error.code not in (429, 500, 502, 503, 504) or attempt == 5:
                        if self.proxy_enabled:
                            raise self.proxy_failure(error) from None
                        raise
                except urllib.error.URLError as error:
                    if attempt == 5:
                        if self.proxy_enabled:
                            raise self.proxy_failure(error) from None
                        raise
                    delay = 3.0 * 2**attempt
                except (http.client.HTTPException, ValueError) as error:
                    if self.proxy_enabled:
                        raise self.proxy_failure(error) from None
                    raise
                # Leave the error handler before an interruptible sleep so an
                # interrupt cannot render a proxy exception's credential context.
                self.sleep(delay)
            with response:
                yield response

    def bytes(self, url, limit=32 * MIB):
        with self.open(url) as response:
            payload = response.read(limit + 1)
        if len(payload) > limit:
            raise CorpusError("Metadata response exceeds the bounded page size")
        return payload


def connect(root):
    db = sqlite3.connect(root / "metadata.sqlite")
    db.execute("PRAGMA journal_mode=WAL")
    db.executescript("""
        CREATE TABLE IF NOT EXISTS papers (id TEXT PRIMARY KEY, body TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS harvests (set_spec TEXT PRIMARY KEY, body TEXT NOT NULL);
    """)
    return db


def parse_oai(payload):
    # ElementTree does not fetch external entities; reject declarations and
    # expansion constructs as well, before parsing remote metadata.
    if b"<!DOCTYPE" in payload.upper() or b"<!ENTITY" in payload.upper():
        raise CorpusError("OAI XML contains a forbidden entity declaration")
    tree = ET.fromstring(payload)
    errors = tree.findall("o:error", NS)
    if errors:
        code = errors[0].get("code")
        if code == "noRecordsMatch":
            return [], "", tree.findtext("o:responseDate", "", NS)
        raise CorpusError(f"OAI error {code}: {errors[0].text}")
    if tree.tag != "{" + NS["o"] + "}OAI-PMH":
        raise CorpusError("Response is not an OAI-PMH document")
    records = []
    for record in tree.findall("o:ListRecords/o:record", NS):
        header = record.find("o:header", NS)
        if header is None:
            raise CorpusError("OAI record has no header")
        identifier = header.findtext("o:identifier", "", NS).removeprefix("oai:arXiv.org:")
        if header.get("status") == "deleted":
            records.append({"id": identifier, "deleted": True})
            continue
        metadata = record.find("o:metadata/a:arXiv", NS)
        if metadata is None:
            raise CorpusError("OAI record has no arXiv metadata")
        get = lambda name: metadata.findtext("a:" + name, "", NS).strip()
        identifier = get("id")
        if not ID_PATTERN.fullmatch(identifier):
            # The checked-in corpus begins in 2020; legacy IDs cannot be selected.
            continue
        records.append({"id": identifier, "created": get("created"), "updated": get("updated"),
                        "title": get("title"), "abstract": get("abstract"), "doi": get("doi"),
                        "categories": get("categories").split(), "license": get("license"),
                        "authors": [{child.tag.split("}")[-1]: child.text or "" for child in author}
                                    for author in metadata.findall("a:authors/a:author", NS)],
                        "datestamp": header.findtext("o:datestamp", "", NS),
                        "arxiv_url": "https://arxiv.org/abs/" + identifier})
    return (records, tree.findtext("o:ListRecords/o:resumptionToken", "", NS).strip(),
            tree.findtext("o:responseDate", "", NS))


def harvest(root, config, http, refresh=False):
    with contextlib.closing(connect(root)) as db:
        for set_spec in sorted(set(config["sets"].values())):
            saved = db.execute("SELECT body FROM harvests WHERE set_spec=?", (set_spec,)).fetchone()
            state = json.loads(saved[0]) if saved else {"from": config["harvest_from"], "token": "", "complete": False}
            if saved and "window_start" not in state and state.get("response_date"):
                # An old checkpoint recorded only its last response. Replaying
                # its original bound is conservative and cannot skip updates.
                state["window_start"] = state["from"] or config["harvest_from"]
            if state["complete"]:
                if not refresh:
                    continue
                state = {"from": state["window_start"], "token": "", "complete": False}
            token_restarted = False
            while not state["complete"]:
                query = {"verb": "ListRecords"}
                if state["token"]:
                    query["resumptionToken"] = state["token"]
                else:
                    query.update(metadataPrefix="arXiv", set=set_spec)
                    if state["from"]:
                        query["from"] = state["from"]
                try:
                    records, token, response_date = parse_oai(http.bytes(OAI + "?" + urllib.parse.urlencode(query)))
                except CorpusError as error:
                    if "badResumptionToken" not in str(error) or token_restarted:
                        raise
                    # Tokens expire daily. Replay the same window, transactionally
                    # upserting IDs; never assume OAI pages sort by submission date.
                    state["token"] = ""
                    token_restarted = True
                    continue
                if not response_date:
                    raise CorpusError("OAI response lacks its incremental harvest timestamp")
                with db:
                    for paper in records:
                        if paper.get("deleted"):
                            db.execute("DELETE FROM papers WHERE id=?", (paper["id"],))
                        else:
                            db.execute("INSERT OR REPLACE INTO papers VALUES (?,?)", (paper["id"], canonical(paper)))
                    response_day = dt.date.fromisoformat(response_date[:10]).isoformat()
                    window_start = min(state.get("window_start", response_day), response_day)
                    state.update(token=token, complete=not token, response_date=response_date,
                                 window_start=window_start)
                    db.execute("INSERT OR REPLACE INTO harvests VALUES (?,?)", (set_spec, canonical(state)))
                print(canonical({"action": "harvest", "set": set_spec, "records": len(records),
                                 "complete": state["complete"]}), flush=True)


def select_papers(papers, config, qualify=None):
    """Stable SHA-256 ordering and disjoint category/year strata within each tier."""
    selected = {}
    unique = {paper["id"]: paper for paper in papers}
    for tier, spec in sorted(config["tiers"].items()):
        strata = {(category, year): [] for category in spec["categories"] for year in spec["years"]}
        for paper in unique.values():
            categories = [category for category in paper["categories"] if category in spec["categories"]]
            if categories and paper["created"]:
                key = (categories[0], int(paper["created"][:4]))
                if key in strata:
                    strata[key].append(paper)
        quotas, extra = divmod(spec["count"], len(strata))
        for index, (stratum, candidates) in enumerate(sorted(strata.items())):
            count = quotas + (index < extra)
            if len(candidates) < count:
                raise CorpusError(f"Insufficient metadata for {tier} {stratum}: {len(candidates)} available, {count} needed")
            ranked = sorted(candidates, key=lambda p: (fingerprint([config["seed"], tier, p["id"]]), p["id"]))
            accepted = 0
            for paper in ranked:
                if accepted == count:
                    break
                if qualify:
                    paper = qualify(paper)
                    if paper is None:
                        continue
                entry = selected.setdefault(paper["id"], {**paper, "tiers": [], "strata": {}})
                entry["tiers"].append(tier)
                entry["strata"][tier] = list(stratum)
                accepted += 1
            if accepted != count:
                raise CorpusError(f"Insufficient available PDFs for {tier} {stratum}: {accepted} available, {count} needed")
    return [selected[key] for key in sorted(selected)]


def selection_metadata(root, config):
    with contextlib.closing(connect(root)) as db:
        for set_spec in set(config["sets"].values()):
            row = db.execute("SELECT body FROM harvests WHERE set_spec=?", (set_spec,)).fetchone()
            if not row or not json.loads(row[0])["complete"]:
                raise CorpusError("Complete every configured metadata harvest before freezing selection")
        papers = [json.loads(row[0]) for row in db.execute("SELECT body FROM papers ORDER BY id")]
    return papers


def remote_problem(remote, identifier, maximum):
    """Return a stable exclusion reason; malformed public inventory is not a PDF."""
    if remote is None:
        return "missing_public_pdf"
    if not isinstance(remote, dict):
        return "invalid_object_record"
    matched = OBJECT_PATTERN.fullmatch(str(remote.get("name", "")))
    if (not matched or matched[2] != identifier or matched[1] != identifier[:4]
            or type(remote.get("version")) is not int or int(matched[3]) != remote["version"]):
        return "invalid_object_identity"
    generation = remote.get("generation")
    if not isinstance(generation, str) or not re.fullmatch(r"[1-9][0-9]{0,30}", generation):
        return "invalid_object_generation"
    if type(remote.get("bytes")) is not int or not 0 < remote["bytes"] <= maximum:
        return "invalid_or_oversized_object_bytes"
    try:
        if not isinstance(remote.get("md5"), str) or len(base64.b64decode(remote["md5"], validate=True)) != 16:
            return "invalid_object_md5"
    except ValueError:
        return "invalid_object_md5"
    return None


def inventory_snapshot(root, proof):
    expected = "availability/inventories/" + proof["sha256"] + ".json"
    if not re.fullmatch(r"[0-9a-f]{64}", proof["sha256"]) or proof["path"] != expected:
        raise CorpusError("Invalid inventory snapshot identity")
    path = root / expected
    if any(parent.is_symlink() for parent in (path, path.parent, path.parent.parent)):
        raise CorpusError("Corpus inventory snapshots cannot use symlinks")
    payload = path.read_bytes()
    if hashlib.sha256(payload).hexdigest() != proof["sha256"]:
        raise CorpusError("Inventory snapshot fingerprint mismatch")
    state = json.loads(payload)
    if state.get("complete") is not True or state.get("token") or not isinstance(state.get("objects"), dict):
        raise CorpusError("Availability requires a complete inventory snapshot")
    return state


class Availability:
    """Durable consulted inputs keep interrupted selection independent of later listings."""

    def __init__(self, root, config, http, metadata_hash, checkpoint):
        self.root, self.config, self.http, self.checkpoint = root, config, http, checkpoint
        if any(path.is_symlink() for path in (root / "availability", root / "availability/inventories",
                                               checkpoint, checkpoint.parent)):
            raise CorpusError("Corpus availability storage cannot use symlinks")
        inputs = {"config_sha256": fingerprint(config), "metadata_sha256": metadata_hash}
        self.state = read_json(checkpoint, {**inputs, "inventories": {}})
        if any(self.state[key] != value for key, value in inputs.items()):
            raise CorpusError("Selection inputs changed during preparation; use a new corpus root")
        self.objects, self.exclusions = {}, {}

    def qualify(self, paper):
        identifier, month = paper["id"], paper["id"][:4]
        if month not in self.objects:
            proof = self.state["inventories"].get(month)
            if proof is None:
                inventory(self.root, month, self.http)
                source = self.root / "inventory" / (month + ".json")
                payload = source.read_bytes().decode("utf-8")
                digest = hashlib.sha256(payload.encode()).hexdigest()
                proof = {"path": "availability/inventories/" + digest + ".json", "sha256": digest,
                         "listing_url": GCS + "/storage/v1/b/arxiv-dataset/o?" +
                         urllib.parse.urlencode({"prefix": f"arxiv/arxiv/pdf/{month}/"})}
                check_space(self.root, len(payload.encode()), int(self.config["free_space_floor_gib"] * GIB))
                immutable_text(self.root / proof["path"], payload)
                self.state["inventories"][month] = proof
                atomic_json(self.checkpoint, self.state)
            self.objects[month] = inventory_snapshot(self.root, proof)["objects"]
        remote = self.objects[month].get(identifier)
        reason = remote_problem(remote, identifier, self.config["max_file_mib"] * MIB)
        digest = self.state["inventories"][month]["sha256"]
        if reason:
            self.exclusions[identifier] = {"id": identifier, "reason": reason,
                                          "inventory_sha256": digest, "remote_pdf": remote}
            return None
        return {**paper, "remote_pdf": remote, "inventory_sha256": digest}

    def evidence(self):
        return {"algorithm": "sha256-strata-public-pdf-v1", "inventories": self.state["inventories"],
                "max_pdf_bytes": self.config["max_file_mib"] * MIB,
                "exclusions": [self.exclusions[key] for key in sorted(self.exclusions)]}


def prepare_selection(root, config, http, checkpoint, original=None):
    papers = selection_metadata(root, config)
    metadata_hash = fingerprint_records(papers)
    if original and metadata_hash != original["metadata_sha256"]:
        raise CorpusError("Metadata changed since the failed selection; recovery needs its original snapshot")
    estimate = sum(spec["count"] for spec in config["tiers"].values()) * config["estimated_pdf_mib"] * MIB
    estimate += config["tiers"]["eval"]["count"] * config["estimated_source_mib"] * MIB
    print(canonical({"action": "selection_preflight", **check_space(root, estimate, int(config["free_space_floor_gib"] * GIB))}), flush=True)
    availability = Availability(root, config, http, metadata_hash, checkpoint)
    if original:
        for paper in original["papers"]:
            availability.qualify(paper)
        if not availability.exclusions:
            raise CorpusError("Legacy selection has no unavailable PDFs; recovery would not repair an availability failure")
    try:
        chosen = select_papers(papers, config, availability.qualify)
    except CorpusError:
        evidence = availability.evidence()
        immutable_text(checkpoint.parent / ("failed-" + fingerprint(evidence) + ".json"), canonical(evidence) + "\n")
        raise
    evidence = availability.evidence()
    selection = {"schema_version": 2, "config_sha256": fingerprint(config), "metadata_sha256": metadata_hash,
                 "selection_sha256": fingerprint(chosen), "papers": chosen,
                 "availability": evidence, "availability_sha256": fingerprint(evidence)}
    validate_selection(selection, config)
    return selection


def select(root, config, http):
    output = root / "selection.json"
    existing = read_json(output)
    if existing:
        if existing["config_sha256"] != fingerprint(config):
            raise CorpusError("Selection already frozen with a different config; use a new corpus root")
        validate_selection(existing, config)
        return existing
    selection = prepare_selection(root, config, http, root / "availability/selection-inputs.json")
    atomic_json(output, selection)
    print(canonical({"action": "select", "papers": len(selection["papers"]), "sha256": selection["selection_sha256"]}), flush=True)
    return selection


def require_zero_artifacts(root):
    for name in ("manifest.jsonl", "manifest.jsonl.partial"):
        path = root / name
        if path.is_symlink() or (path.exists() and (name.endswith(".partial") or path.read_bytes().strip())):
            raise CorpusError("Selection recovery requires zero manifest entries and no pending manifest")
    for name in ("pdf", "source"):
        path = root / name
        if path.is_symlink() or (path.exists() and (not path.is_dir() or any(path.iterdir()))):
            raise CorpusError("Selection recovery refuses artifacts, receipts and partial downloads")


def recover_selection(root, config, http, reason):
    require_zero_artifacts(root)
    if not reason.strip() or len(reason.encode()) > 4096:
        raise CorpusError("Recovery requires a nonempty failure reason of at most 4096 bytes")
    output = root / "selection.json"
    raw = output.read_bytes().decode("utf-8")
    original = json.loads(raw)
    validate_selection(original, config)
    # If publication succeeded just before a crash, validate the immutable staged
    # replacement and return it; never perform a second selection.
    recovery = original.get("recovery")
    old_hash = recovery["original_file_sha256"] if recovery else hashlib.sha256(raw.encode()).hexdigest()
    if not re.fullmatch(r"[0-9a-f]{64}", old_hash):
        raise CorpusError("Invalid recovery archive identity")
    archive = root / "selection-recovery" / old_hash
    if any(path.is_symlink() for path in (archive, archive.parent)):
        raise CorpusError("Corpus recovery archives cannot use symlinks")
    replacement_path = archive / "replacement.json"
    if any((archive / name).is_symlink() for name in ("replacement.json", "original-selection.json", "request.json", "inputs.json")):
        raise CorpusError("Corpus recovery archives cannot use symlinks")
    if recovery:
        if not replacement_path.is_file() or replacement_path.read_bytes() != raw.encode():
            raise CorpusError("Published selection differs from staged recovery archive")
        archived = archive / "original-selection.json"
        if hashlib.sha256(archived.read_bytes()).hexdigest() != old_hash:
            raise CorpusError("Original recovery archive fingerprint mismatch")
        if read_json(archive / "request.json") != recovery or recovery["reason"] != reason:
            raise CorpusError("Recovery reason differs from the archived request")
        validate_availability(root, original)
        return original
    if original.get("schema_version") != 1:
        raise CorpusError("Recovery is limited to legacy metadata-only selections")
    immutable_text(archive / "original-selection.json", raw)
    request = {"schema_version": 1, "original_file_sha256": old_hash,
               "original_selection_sha256": original["selection_sha256"], "reason": reason}
    immutable_text(archive / "request.json", canonical(request) + "\n")
    replacement = read_json(replacement_path)
    if replacement is None:
        replacement = prepare_selection(root, config, http, archive / "inputs.json", original)
        replacement["recovery"] = request
        immutable_text(replacement_path, canonical(replacement) + "\n")
    elif replacement.get("recovery") != request:
        raise CorpusError("Staged recovery does not match the archived request")
    validate_selection(replacement, config)
    validate_availability(root, replacement)
    require_zero_artifacts(root)
    atomic_json(output, replacement)
    print(canonical({"action": "recover_selection", "original_sha256": old_hash,
                     "papers": len(replacement["papers"]), "sha256": replacement["selection_sha256"]}), flush=True)
    return replacement


def gcs_objects(payload, month):
    objects = {}
    for item in payload.get("items", []):
        matched = OBJECT_PATTERN.fullmatch(item.get("name", ""))
        if not matched or matched[1] != month:
            continue
        identifier, version = matched[2], int(matched[3])
        if identifier[:4] != month:
            raise CorpusError("GCS object has inconsistent paper/month identity")
        if identifier not in objects or version > objects[identifier]["version"]:
            size = item.get("size")
            if isinstance(size, str) and re.fullmatch(r"[0-9]{1,20}", size):
                size = int(size)
            objects[identifier] = {"version": version, "name": item["name"], "bytes": size,
                                   "md5": item.get("md5Hash"), "generation": item.get("generation")}
    return objects


def inventory(root, month, http):
    path = root / "inventory" / (month + ".json")
    if path.is_symlink():
        raise CorpusError("Corpus inventory cannot use symlinks")
    state = read_json(path, {"complete": False, "token": "", "objects": {}})
    while not state["complete"]:
        query = {"prefix": f"arxiv/arxiv/pdf/{month}/", "maxResults": 1000,
                 "fields": "items(name,size,md5Hash,generation),nextPageToken"}
        if state["token"]:
            query["pageToken"] = state["token"]
        payload = json.loads(http.bytes(GCS + "/storage/v1/b/arxiv-dataset/o?" + urllib.parse.urlencode(query)))
        for identifier, item in gcs_objects(payload, month).items():
            previous = state["objects"].get(identifier)
            if not previous or item["version"] > previous["version"]:
                state["objects"][identifier] = item
        state.update(token=payload.get("nextPageToken", ""), complete=not payload.get("nextPageToken"))
        atomic_json(path, state)
    return state["objects"]


def digest_file(path):
    sha, md5, count = hashlib.sha256(), hashlib.md5(usedforsecurity=False), 0
    with path.open("rb") as source:
        while chunk := source.read(MIB):
            sha.update(chunk)
            md5.update(chunk)
            count += len(chunk)
    return {"sha256": sha.hexdigest(), "md5": base64.b64encode(md5.digest()).decode(), "bytes": count}


def verified(path, receipt):
    return bool(receipt and path.is_file() and not path.is_symlink()
                and digest_file(path) == {key: receipt[key] for key in ("sha256", "md5", "bytes")})


def fetch_file(path, url, kind, http, floor, maximum, expected=None):
    if path.parent.is_symlink() or path.is_symlink():
        raise CorpusError("Corpus artifacts cannot use symlinks")
    receipt_path = path.with_name(path.name + ".verified.json")
    if receipt_path.is_symlink():
        raise CorpusError("Corpus receipts cannot use symlinks")
    receipt = read_json(receipt_path)
    if receipt and receipt.get("url") == url and verified(path, receipt):
        if not expected or all(receipt[key] == expected[key] for key in ("md5", "bytes")):
            return receipt, False
    if path.exists():
        # Never silently replace local corpus content whose provenance is absent
        # or whose bytes changed. A user can inspect it and choose a new root.
        raise CorpusError(f"Existing file failed verification; leaving it untouched: {path}")
    path.parent.mkdir(parents=True, exist_ok=True)
    partial = path.with_name(path.name + ".partial")
    partial.unlink(missing_ok=True)
    sha, md5, size, header = hashlib.sha256(), hashlib.md5(usedforsecurity=False), 0, b""
    try:
        with http.open(url) as response, partial.open("xb") as out:
            length = response.headers.get("Content-Length")
            length = int(length) if length is not None else None
            if length is not None and length > maximum:
                raise CorpusError("Download exceeds the file size cap")
            while chunk := response.read(MIB):
                size += len(chunk)
                if size > maximum:
                    raise CorpusError("Download exceeds the file size cap")
                check_space(path.parent, len(chunk), floor)
                if len(header) < 512:
                    header += chunk[:512 - len(header)]
                sha.update(chunk)
                md5.update(chunk)
                out.write(chunk)
            if not size or (length is not None and size != length):
                raise CorpusError("Incomplete or empty download")
            hashes = {"sha256": sha.hexdigest(), "md5": base64.b64encode(md5.digest()).decode(), "bytes": size}
            if expected and any(hashes[key] != expected[key] for key in ("md5", "bytes")):
                raise CorpusError("Download hash or size mismatches the GCS inventory")
            if kind == "pdf" and not header.startswith(b"%PDF-"):
                raise CorpusError("Downloaded PDF has no PDF header")
            source_format = None
            if kind == "source":
                source_format = ("gzip" if header.startswith(b"\x1f\x8b") else "pdf" if header.startswith(b"%PDF-")
                                 else "tar" if header[257:262] == b"ustar" else "tex" if b"\\" in header else None)
                if not source_format or b"<html" in header.lower() or b"<!doctype html" in header.lower():
                    raise CorpusError("Response is not a recognized arXiv source format")
            out.flush()
            os.fsync(out.fileno())
        receipt = {**hashes, "url": url, "fetched_at": now(), "kind": kind, "source_format": source_format}
        # Receipt precedes rename so an interrupted manifest update can resume
        # without downloading an already verified source a second time.
        atomic_json(receipt_path, receipt)
        partial.replace(path)
        return receipt, True
    finally:
        partial.unlink(missing_ok=True)


def write_manifest(root, entries):
    atomic_text(root / "manifest.jsonl", "".join(canonical(entries[key]) + "\n" for key in sorted(entries)))


def read_manifest(root):
    path = root / "manifest.jsonl"
    if not path.exists():
        return {}
    with path.open(encoding="utf-8") as source:
        rows = [json.loads(line) for line in source if line.strip()]
    if len({row["id"] for row in rows}) != len(rows):
        raise CorpusError("Manifest contains duplicate paper IDs")
    return {row["id"]: row for row in rows}


def validate_selection(selection, config):
    if not selection or selection["config_sha256"] != fingerprint(config):
        raise CorpusError("Freeze a selection with this config before downloading")
    chosen = selection["papers"]
    if selection.get("selection_sha256") != fingerprint(chosen):
        raise CorpusError("Frozen selection fingerprint mismatches its records")
    if len({paper["id"] for paper in chosen}) != len(chosen):
        raise CorpusError("Selection contains duplicate paper IDs")
    for paper in chosen:
        if not ID_PATTERN.fullmatch(paper["id"]) or not paper["tiers"] or not set(paper["tiers"]) <= {"eval", "scale"}:
            raise CorpusError("Invalid paper ID or tier in frozen selection")
    if selection.get("schema_version", 1) not in (1, 2):
        raise CorpusError("Unsupported frozen selection schema")
    if selection.get("schema_version") == 2:
        if fingerprint(selection["availability"]) != selection.get("availability_sha256"):
            raise CorpusError("Availability evidence fingerprint mismatch")
        if selection["availability"]["max_pdf_bytes"] != config["max_file_mib"] * MIB:
            raise CorpusError("Availability size cap differs from the configured cap")
        counts = {}
        for paper in chosen:
            if paper["tiers"] != sorted(set(paper["tiers"])):
                raise CorpusError("Frozen tiers must be unique and sorted")
            problem = remote_problem(paper["remote_pdf"], paper["id"], config["max_file_mib"] * MIB)
            if problem:
                raise CorpusError("Frozen PDF is invalid: " + problem)
            for tier in paper["tiers"]:
                spec = config["tiers"][tier]
                categories = [c for c in paper["categories"] if c in spec["categories"]]
                expected = [categories[0], int(paper["created"][:4])] if categories else None
                if expected is None or paper["strata"].get(tier) != expected or expected[1] not in spec["years"]:
                    raise CorpusError("Frozen stratum differs from original metadata")
                key = (tier, *expected)
                counts[key] = counts.get(key, 0) + 1
        for tier, spec in config["tiers"].items():
            strata = sorted((c, y) for c in spec["categories"] for y in spec["years"])
            quota, extra = divmod(spec["count"], len(strata))
            if any(counts.get((tier, *s), 0) != quota + (i < extra) for i, s in enumerate(strata)):
                raise CorpusError("Frozen available selection does not meet exact stratum quotas")


def validate_availability(root, selection):
    if selection.get("schema_version") != 2:
        return
    evidence = selection["availability"]
    if evidence.get("algorithm") != "sha256-strata-public-pdf-v1":
        raise CorpusError("Unsupported availability algorithm")
    for month, proof in evidence["inventories"].items():
        if not re.fullmatch(r"[0-9]{4}", month):
            raise CorpusError("Invalid inventory month")
        expected_url = GCS + "/storage/v1/b/arxiv-dataset/o?" + urllib.parse.urlencode({"prefix": f"arxiv/arxiv/pdf/{month}/"})
        if proof["listing_url"] != expected_url:
            raise CorpusError("Inventory provenance has an unexpected listing URL")
        objects = inventory_snapshot(root, proof)["objects"]
        rows = [p for p in selection["papers"] if p["id"][:4] == month]
        rows += [p for p in evidence["exclusions"] if p["id"][:4] == month]
        for paper in rows:
            if paper["inventory_sha256"] != proof["sha256"] or paper["remote_pdf"] != objects.get(paper["id"]):
                raise CorpusError("Frozen object differs from preserved inventory evidence")
    if any(p["id"][:4] not in evidence["inventories"] for p in selection["papers"] + evidence["exclusions"]):
        raise CorpusError("Missing inventory evidence for a frozen candidate")
    for paper in evidence["exclusions"]:
        if paper["reason"] != remote_problem(paper["remote_pdf"], paper["id"], evidence["max_pdf_bytes"]):
            raise CorpusError("Exclusion reason differs from preserved object evidence")
    recovery = selection.get("recovery")
    if recovery:
        old_hash = recovery["original_file_sha256"]
        if not re.fullmatch(r"[0-9a-f]{64}", old_hash):
            raise CorpusError("Invalid recovery archive identity")
        archive = root / "selection-recovery" / old_hash
        paths = [archive / name for name in ("original-selection.json", "request.json", "replacement.json")]
        if any(path.is_symlink() for path in [archive.parent, archive, *paths]):
            raise CorpusError("Corpus recovery archives cannot use symlinks")
        original = paths[0].read_bytes()
        if (hashlib.sha256(original).hexdigest() != old_hash or read_json(paths[1]) != recovery
                or read_json(paths[2]) != selection
                or json.loads(original)["selection_sha256"] != recovery["original_selection_sha256"]):
            raise CorpusError("Recovery archive differs from frozen provenance")


def validate_entry(entry, paper, selection_hash):
    remote = entry["remote_pdf"]
    matched = OBJECT_PATTERN.fullmatch(remote["name"])
    if (entry["id"] != paper["id"] or entry["tiers"] != paper["tiers"]
            or any(entry.get(key) != paper.get(key) for key in ("categories", "strata", "arxiv_url", "license"))
            or entry["selection_sha256"] != selection_hash or not matched
            or matched[2] != paper["id"] or matched[1] != paper["id"][:4]
            or int(matched[3]) != entry["version"] or remote["version"] != entry["version"]
            or not str(remote["generation"]).isdigit() or remote["bytes"] <= 0):
        raise CorpusError("Manifest identity differs from pinned paper selection")
    if "remote_pdf" in paper and remote != paper["remote_pdf"]:
        raise CorpusError("Manifest PDF differs from availability-pinned selection")
    if len(base64.b64decode(remote["md5"], validate=True)) != 16:
        raise CorpusError("Pinned PDF has an invalid MD5 hash")


def artifact_location(entry, kind):
    stem = f"{entry['id']}v{entry['version']}"
    if kind == "pdf":
        remote = entry["remote_pdf"]
        return ("pdf/" + stem + ".pdf",
                GCS + "/arxiv-dataset/" + remote["name"] + "?generation=" + str(remote["generation"]))
    return "source/" + stem + ".src", EXPORT + stem


def validate_receipt(entry, kind, receipt):
    path, url = artifact_location(entry, kind)
    if receipt["path"] != path or receipt["url"] != url or receipt["kind"] != kind:
        raise CorpusError("Artifact path, URL or kind differs from pinned paper version")
    if kind == "pdf" and any(receipt[key] != entry["remote_pdf"][key] for key in ("md5", "bytes")):
        raise CorpusError("PDF receipt differs from pinned GCS hash or size")
    if (not re.fullmatch(r"[0-9a-f]{64}", receipt["sha256"])
            or len(base64.b64decode(receipt["md5"], validate=True)) != 16 or receipt["bytes"] <= 0):
        raise CorpusError("Artifact receipt has invalid hashes or size")
    if kind == "source" and receipt["source_format"] not in {"gzip", "tar", "tex", "pdf"}:
        raise CorpusError("Source receipt has an unknown format")
    if not dt.datetime.fromisoformat(receipt["fetched_at"]).tzinfo:
        raise CorpusError("Artifact receipt has no timezone for its fetch timestamp")


def download(root, config, http, tier="all", sources=True, limit=None):
    selection = read_json(root / "selection.json")
    validate_selection(selection, config)
    validate_availability(root, selection)
    papers = [paper for paper in selection["papers"] if tier == "all" or tier in paper["tiers"]]
    if limit is not None:
        papers = papers[:limit]
    entries = read_manifest(root)
    floor = int(config["free_space_floor_gib"] * GIB)
    estimate = sum((0 if entries.get(p["id"], {}).get("pdf") else config["estimated_pdf_mib"] * MIB)
                   + (config["estimated_source_mib"] * MIB if sources and "eval" in p["tiers"]
                      and not entries.get(p["id"], {}).get("source") else 0) for p in papers)
    print(canonical({"action": "disk_preflight", **check_space(root, estimate, floor)}), flush=True)
    # Pin versions, remote object generations, sizes and MD5 before any PDF
    # starts. A resumed run reuses this snapshot even if the bucket later changes.
    inventories = {}
    for paper in papers:
        identifier = paper["id"]
        entry = entries.get(identifier)
        if not entry:
            month = identifier[:4]
            remote = paper.get("remote_pdf")
            if remote is None:
                if month not in inventories:
                    inventories[month] = inventory(root, month, http)
                remote = inventories[month].get(identifier)
            if not remote:
                raise CorpusError(f"Selected PDF unavailable in public bucket: {identifier}")
            entries[identifier] = {key: paper[key] for key in
                                   ("id", "categories", "tiers", "strata", "arxiv_url", "license") if key in paper}
            entries[identifier].update(version=remote["version"], remote_pdf=remote,
                                       selection_sha256=selection["selection_sha256"], pdf=None, source=None)
    for paper in papers:
        validate_entry(entries[paper["id"]], paper, selection["selection_sha256"])
    write_manifest(root, entries)
    exact = sum(entries[p["id"]]["remote_pdf"]["bytes"] for p in papers if not entries[p["id"]]["pdf"])
    exact += sum(config["estimated_source_mib"] * MIB for p in papers if sources and "eval" in p["tiers"]
                 and not entries[p["id"]]["source"])
    print(canonical({"action": "disk_preflight_pinned", **check_space(root, exact, floor)}), flush=True)
    try:
        for index, paper in enumerate(papers):
            entry = entries[paper["id"]]
            remote = entry["remote_pdf"]
            relative, url = artifact_location(entry, "pdf")
            path = root / relative
            receipt, fetched = fetch_file(path, url, "pdf", http, floor, config["max_file_mib"] * MIB, remote)
            entry["pdf"] = {**receipt, "path": str(path.relative_to(root))}
            print(canonical({"action": "pdf", "id": entry["id"], "downloaded": fetched}), flush=True)
            if sources and "eval" in entry["tiers"]:
                relative, url = artifact_location(entry, "source")
                path = root / relative
                receipt, fetched = fetch_file(path, url, "source", http, floor, config["max_file_mib"] * MIB)
                entry["source"] = {**receipt, "path": str(path.relative_to(root))}
                print(canonical({"action": "source", "id": entry["id"], "downloaded": fetched}), flush=True)

            if (index + 1) % 25 == 0:
                write_manifest(root, entries)
    finally:
        # Receipts are durable per artifact; batch JSONL projection updates to
        # avoid rewriting the whole 10k manifest twice per selected paper.
        write_manifest(root, entries)


def status(root, verify=False, config=None):
    entries = read_manifest(root)
    selection = read_json(root / "selection.json", {"papers": []})
    result = {"selected": len(selection["papers"]), "manifest_entries": len(entries),
              "pdfs": 0, "sources": 0, "bytes": 0, "tiers": {}, "problems": []}
    if verify and selection["papers"]:
        config = config or load_config(Path(__file__).with_name("selection.json"))
        try:
            validate_selection(selection, config)
            validate_availability(root, selection)
        except (CorpusError, ValueError, KeyError, TypeError) as error:
            result["problems"].append("Invalid frozen selection: " + str(error))
            print(canonical(result), flush=True)
            return False
    selected = {paper["id"]: paper for paper in selection["papers"]}
    available = {}
    for identifier, entry in entries.items():
        available[identifier] = set()
        if verify:
            try:
                if identifier not in selected:
                    raise CorpusError("Manifest paper is absent from frozen selection")
                validate_entry(entry, selected[identifier], selection["selection_sha256"])
            except (CorpusError, ValueError, KeyError, TypeError) as error:
                result["problems"].append(f"{identifier}: invalid manifest: {error}")
                continue
        for kind in ("pdf", "source"):
            receipt = entry.get(kind)
            if not receipt:
                continue
            try:
                relative = Path(receipt["path"])
                if relative.is_absolute() or ".." in relative.parts or not (root / relative).resolve().is_relative_to(root):
                    raise CorpusError("Manifest path leaves corpus root")
                if verify:
                    validate_receipt(entry, kind, receipt)
                    if not verified(root / relative, receipt):
                        raise CorpusError("Artifact bytes differ from its verified receipt")
            except (CorpusError, OSError, ValueError, KeyError, TypeError) as error:
                result["problems"].append(f"{identifier}: invalid {kind}: {error}")
                continue
            available[identifier].add(kind)
            result[kind + "s"] += 1
            result["bytes"] += receipt["bytes"]
    for tier in ("eval", "scale"):
        chosen = [p for p in selection["papers"] if tier in p["tiers"]]
        result["tiers"][tier] = {"selected": len(chosen), "pdfs": 0, "sources": 0}
        if verify and config and len(chosen) != config["tiers"][tier]["count"]:
            result["problems"].append(f"{tier}: selected count does not match configured target")
        for paper in chosen:
            for kind in ("pdf", "source"):
                if kind in available.get(paper["id"], set()):
                    result["tiers"][tier][kind + "s"] += 1
                elif verify and (kind == "pdf" or tier == "eval"):
                    result["problems"].append(f"{paper['id']}: missing {kind}")
    if verify and not selection["papers"]:
        result["problems"].append("No frozen selection")
    print(canonical(result), flush=True)
    return not result["problems"]


def load_config(path):
    config = read_json(path)
    if not config or config.get("schema_version") != 1:
        raise CorpusError("Unsupported or missing selection config")
    for key in ("free_space_floor_gib", "estimated_pdf_mib", "estimated_source_mib", "max_file_mib"):
        value = config[key]
        if not isinstance(value, (int, float)) or not math.isfinite(value) or value <= 0:
            raise CorpusError(f"Config {key} must be a positive finite number")
    if set(config["tiers"]) != {"eval", "scale"}:
        raise CorpusError("Config must define eval and scale tiers")
    for spec in config["tiers"].values():
        if not isinstance(spec["count"], int) or spec["count"] <= 0 or not spec["years"] or not spec["categories"]:
            raise CorpusError("Tier needs a positive count, years and categories")
        if len(set(spec["years"])) != len(spec["years"]) or len(set(spec["categories"])) != len(spec["categories"]):
            raise CorpusError("Tier strata cannot contain duplicates")
        if not set(spec["categories"]) <= set(config["sets"]):
            raise CorpusError("Every selected category needs an OAI set")
    return config


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", default=os.environ.get("LYSILOGY_CORPUS", "~/Corpora/arxiv"))
    parser.add_argument("--config", type=Path, default=Path(__file__).with_name("selection.json"))
    parser.add_argument("--proxy-env", choices=PROXY_ENV_VARS,
                        help="Explicitly use the HTTPS proxy from this environment variable; direct transport is the default")
    commands = parser.add_subparsers(dest="command", required=True)
    harvest_parser = commands.add_parser("harvest", help="Resume OAI metadata harvest")
    harvest_parser.add_argument("--refresh", action="store_true", help="Fetch updates since each completed harvest")
    commands.add_parser("select", help="Freeze deterministic stratified selection")
    recovery_parser = commands.add_parser("recover-selection", help="Explicitly repair a legacy zero-artifact selection")
    recovery_parser.add_argument("--reason", required=True, help="Observed failure retained in the immutable recovery archive")
    for name in ("download", "run"):
        sub = commands.add_parser(name, help="Download selected papers" if name == "download" else "Harvest, select, download and verify")
        sub.add_argument("--tier", choices=("all", "eval", "scale"), default="all")
        sub.add_argument("--no-sources", action="store_true")
        sub.add_argument("--limit", type=int, help="Bound a live verification run")
    commands.add_parser("verify", help="Rehash every required selected artifact")
    commands.add_parser("status", help="Show progress without reading artifact bytes")
    args = parser.parse_args(argv)
    try:
        root = validate_root(args.root)
        config = load_config(args.config)
        if hasattr(args, "limit") and args.limit is not None and args.limit <= 0:
            raise CorpusError("--limit must be positive")
        if args.command in {"status", "verify"}:
            return 0 if status(root, args.command == "verify", config) else 1
        root.mkdir(parents=True, exist_ok=True)
        for name in (".corpus.lock", "metadata.sqlite", "metadata.sqlite-wal", "metadata.sqlite-shm",
                     "selection.json", "manifest.jsonl", "inventory", "pdf", "source", "availability", "selection-recovery"):
            if (root / name).is_symlink():
                raise CorpusError("Corpus storage cannot use symlinks: " + name)
        with (root / ".corpus.lock").open("a+") as lock:
            try:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError as error:
                raise CorpusError("Another corpus mutation is already running at this root") from error
            http = Http(proxy_env=args.proxy_env)
            if args.command in {"harvest", "run"}:
                harvest(root, config, http, getattr(args, "refresh", False))
            if args.command in {"select", "run"}:
                select(root, config, http)
            if args.command == "recover-selection":
                recover_selection(root, config, http, args.reason)
            if args.command in {"download", "run"}:
                download(root, config, http, args.tier, not args.no_sources, args.limit)
            if args.command == "run" and args.tier == "all" and not args.no_sources and args.limit is None:
                return 0 if status(root, verify=True, config=config) else 1
        return 0
    except (CorpusError, OSError, ValueError, KeyError, TypeError, OverflowError, ET.ParseError, sqlite3.Error) as error:
        print(f"corpus: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
