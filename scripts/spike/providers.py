#!/usr/bin/env python3
"""Show live OpenRouter endpoints for a model, cheapest first.

Prints quantization, price per million tokens, uptime and the parameters each
host supports, so a provider can be chosen in advance.

Usage: python3 -B scripts/spike/providers.py [model]
"""
import json
import sys
import urllib.request

MODEL = sys.argv[1] if len(sys.argv) > 1 else "deepseek/deepseek-v4.1-flash"


def main():
    url = f"https://openrouter.ai/api/v1/models/{MODEL}/endpoints"
    with urllib.request.urlopen(url, timeout=30) as response:
        endpoints = json.load(response)["data"]["endpoints"]
    rows = sorted(endpoints, key=lambda e: float(e.get("pricing", {}).get("completion") or 0))
    print(f"{MODEL}: {len(rows)} endpoints")
    print(f"{'provider':<14} {'tag':<24} {'quant':<8} {'out$/M':>8} {'in$/M':>7} "
          f"{'up5m':>5} {'up1d':>5}  caps")
    for endpoint in rows:
        price = endpoint.get("pricing", {})
        parameters = endpoint.get("supported_parameters") or []
        caps = "json" if {"response_format", "structured_outputs"} & set(parameters) else "-"
        caps += "+reasoning" if "reasoning" in parameters else ""
        def uptime(key):
            value = endpoint.get(key)
            return f"{value:.0f}" if isinstance(value, (int, float)) else "-"
        print(f"{endpoint.get('provider_name', '?'):<14} {endpoint.get('tag') or '?':<24} "
              f"{endpoint.get('quantization') or '?':<8} "
              f"{float(price.get('completion') or 0) * 1e6:>8.3f} "
              f"{float(price.get('prompt') or 0) * 1e6:>7.3f} "
              f"{uptime('uptime_last_5m'):>5} {uptime('uptime_last_1d'):>5}  {caps}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
