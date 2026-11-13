#!/usr/bin/env python3
"""Write the five frozen PR14870 review pack controls (no oracle/game execution).

H146_REVIEW_CASES points the ignored loaded-Game regression at this output.
Expectations and the 32646-character literal were admitted by pinned Java 26.3;
this generator does not consult the native codec or reconstruct old captures.
"""
import argparse
import json
from pathlib import Path


def cases():
    out = []

    def add(name, description, valid):
        raw = json.dumps(description, separators=(",", ":")).replace(
            '"__huge__"', "1e2147483647"
        )
        out.append({"name": name, "description": raw, "valid": valid})

    for name, page, width, valid in [
        ("single_option_width_error_nested_exception", "__huge__", 0, False),
        ("single_option_width_error_ordinary_control", 1, 0, True),
        ("single_option_valid_control", 1, 200, True),
    ]:
        display = {"text": "o", "click_event": {"action": "change_page", "page": page}}
        control = {"type": "single_option", "key": "k", "label": "l", "width": width,
                   "options": [{"id": "a", "display": display}]}
        dialog = {"type": "notice", "title": "t", "inputs": [control]}
        separator = {"text": "s", "click_event": {"action": "show_dialog", "dialog": dialog}}
        add(name, {"nbt": "x", "storage": "test:x", "separator": separator}, valid)

    for name, patch in [
        ("set_then_remove", {"custom_name": "n", "!custom_name": {}}),
        ("remove_then_set", {"!custom_name": {}, "minecraft:custom_name": "n"}),
    ]:
        page = {"text": "x" * 32646, "hover_event": {
            "action": "show_item", "id": "minecraft:stone", "components": patch}}
        book = {"title": "t", "author": "a", "pages": [page]}
        add(name + "_restricted_page", {"text": "p", "hover_event": {
            "action": "show_item", "id": "stone", "components": {"written_book_content": book}}}, True)
    return out


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.output.write_text(json.dumps(cases(), indent=2), encoding="utf-8")
