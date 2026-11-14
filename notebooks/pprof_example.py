"""Presentation for authored pprof comparisons with explicit denominators."""

import hashlib
import json
from pathlib import Path

import matplotlib.pyplot as plt
import numpy as np
from vgi_notebooks import finish

SOURCE = "Source: authored pprof fixtures and operation counts · VGI pprof v0.4.0 · no real workload measured"
COLORS = {
    "parse": "#267358",
    "serialize": "#456fba",
    "legacy": "#c57b40",
    "new_cache": "#8b6aa8",
    "[unsymbolized]": "#8b9790",
}


def validate_basis(value):
    """Validate the visible comparison-basis parameter."""
    if type(value) is not bool:
        raise ValueError("PER_OPERATION must be True or False.")


def verify_fixtures():
    """Require the three original authored profile byte images."""
    manifest = json.loads(Path("fixtures/provenance.json").read_text())
    for name, expected in manifest["files"].items():
        actual = hashlib.sha256((Path("fixtures") / name).read_bytes()).hexdigest()
        if actual != expected["sha256"]:
            raise ValueError(f"Fixture hash differs from recorded provenance: {name}")


def unit(per_operation):
    """Label the analytical denominator explicitly."""
    return (
        "CPU milliseconds per completed operation" if per_operation else "Captured CPU milliseconds"
    )


def parts(data, per_operation):
    """Plot exclusive leaf parts on the selected comparison basis."""
    wide = data.pivot(index="cohort", columns="leaf_label", values="selected_value").fillna(0)
    wide = wide.reindex(["baseline", "candidate"])
    fig, ax = plt.subplots(figsize=(11.5, 6.4))
    left = np.zeros(2)
    for label, color in COLORS.items():
        values = wide[label].to_numpy()
        ax.barh(wide.index, values, left=left, color=color, height=0.45, label=label)
        left += values
    for y, total in enumerate(left):
        ax.annotate(
            f"{total:.3g}",
            (total, y),
            xytext=(7, 0),
            textcoords="offset points",
            va="center",
            fontsize=12,
            weight="bold",
        )
    ax.invert_yaxis()
    ax.set_xlim(0, float(left.max()) * 1.18)
    ax.set_xlabel(unit(per_operation))
    ax.grid(axis="y", visible=False)
    fig.legend(
        *ax.get_legend_handles_labels(),
        loc="lower center",
        bbox_to_anchor=(0.55, 0.76),
        ncol=5,
        frameon=False,
    )
    finish(
        fig,
        "Declare the denominator before comparing",
        f"{'Per completed operation' if per_operation else 'Whole capture totals'} · leaf attribution · unknown symbols retained",
        SOURCE,
        name="01-cpu-comparison",
    )
    return left


def differences(data, per_operation):
    """Retain both one-sided leaf categories and aggregate unknown symbols."""
    fig, ax = plt.subplots(figsize=(11.5, 6.3))
    bars = ax.barh(
        data.leaf_label,
        data.delta,
        color=["#267358" if x < 0 else "#c57b40" for x in data.delta],
        height=0.52,
    )
    span = max(float(data.delta.abs().max()), 0.001)
    for bar, value in zip(bars, data.delta, strict=True):
        ax.annotate(
            f"{value:+.3g}",
            (value, bar.get_y() + bar.get_height() / 2),
            xytext=(7 if value >= 0 else -7, 0),
            textcoords="offset points",
            va="center",
            ha="left" if value >= 0 else "right",
            fontsize=11,
        )
    ax.axvline(0, color="#253d31", linewidth=1.3)
    ax.set_xlim(-span * 1.55, span * 1.55)
    ax.invert_yaxis()
    ax.set_xlabel(f"Candidate − baseline · {unit(per_operation).lower()}")
    ax.grid(axis="y", visible=False)
    finish(
        fig,
        "Keep categories observed on either side",
        "Green: lower attributed value · amber: higher · signs do not establish real performance effects",
        SOURCE,
        name="02-leaf-difference",
    )
