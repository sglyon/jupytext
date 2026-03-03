#!/usr/bin/env python3
"""Generate synthetic notebooks for benchmarking.

Produces small / medium / large / xlarge .ipynb files so that both the
Python and Rust implementations can be exercised on identical inputs.
"""

import json, os, sys, textwrap

def make_cell(cell_type, source, **extra):
    cell = {"cell_type": cell_type, "source": source, "metadata": {}}
    if cell_type == "code":
        cell["execution_count"] = None
        cell["outputs"] = []
    cell.update(extra)
    return cell

def make_notebook(cells, kernel="python3", lang="python"):
    return {
        "nbformat": 4,
        "nbformat_minor": 5,
        "metadata": {
            "kernelspec": {
                "display_name": "Python 3",
                "language": lang,
                "name": kernel,
            },
            "language_info": {
                "name": lang,
                "version": "3.11.0",
                "file_extension": ".py",
                "mimetype": "text/x-python",
            },
        },
        "cells": cells,
    }

def gen_small():
    """3 cells, ~500 bytes"""
    return make_notebook([
        make_cell("markdown", "# Hello\n\nA tiny notebook."),
        make_cell("code", "x = 1\nprint(x)"),
        make_cell("markdown", "Done."),
    ])

def gen_medium():
    """~20 cells, ~5 KB"""
    cells = [make_cell("markdown", "# Medium benchmark notebook\n\nThis notebook has 20 cells.")]
    for i in range(19):
        if i % 3 == 0:
            cells.append(make_cell("markdown", f"## Section {i}\n\nSome explanatory text for section {i}."))
        else:
            code = "\n".join([
                f"def func_{i}(x):",
                f'    """Docstring for func_{i}."""',
                f"    result = x ** {i + 2}",
                f"    return result",
                "",
                f"print(func_{i}({i}))",
            ])
            cells.append(make_cell("code", code))
    return make_notebook(cells)

def gen_large():
    """~100 cells, ~50 KB"""
    cells = [make_cell("markdown", "# Large benchmark notebook\n\nThis notebook has 100 cells for performance testing.")]
    for i in range(99):
        if i % 4 == 0:
            md = f"## Chapter {i // 4 + 1}\n\n"
            md += f"This chapter covers topic {i // 4 + 1}. " * 10
            md += "\n\n- Item 1\n- Item 2\n- Item 3"
            cells.append(make_cell("markdown", md))
        elif i % 4 == 3:
            cells.append(make_cell("raw", f"Raw cell {i}\nWith multiple lines\nFor testing purposes."))
        else:
            lines = [f"# Cell {i}", "import numpy as np", ""]
            for j in range(8):
                lines.append(f"def process_{i}_{j}(data):")
                lines.append(f'    """Process data batch {j}."""')
                lines.append(f"    return [x * {j + 1} for x in data]")
                lines.append("")
            lines.append(f"result_{i} = process_{i}_0(list(range(100)))")
            lines.append(f"print(f'Result: {{len(result_{i})}}')")
            cells.append(make_cell("code", "\n".join(lines)))
    return make_notebook(cells)

def gen_xlarge():
    """~500 cells, ~250 KB"""
    cells = [make_cell("markdown", "# Extra-large benchmark notebook\n\n500 cells for stress testing.")]
    for i in range(499):
        if i % 5 == 0:
            md_lines = [f"## Section {i // 5 + 1}", ""]
            md_lines.append("Lorem ipsum dolor sit amet. " * 20)
            md_lines.append("")
            md_lines.append("| Col A | Col B | Col C |")
            md_lines.append("|-------|-------|-------|")
            for r in range(5):
                md_lines.append(f"| {r} | {r*2} | {r*3} |")
            cells.append(make_cell("markdown", "\n".join(md_lines)))
        elif i % 5 == 4:
            cells.append(make_cell("raw", f"---\ntitle: Raw block {i}\ntags: [bench, test]\n---"))
        else:
            lines = [f"# Code cell {i}"]
            for k in range(12):
                lines.append(f"class Worker{i}_{k}:")
                lines.append(f'    """Worker class {k} in cell {i}."""')
                lines.append(f"    def __init__(self, n={k}):")
                lines.append(f"        self.n = n")
                lines.append(f"    def run(self):")
                lines.append(f"        return self.n ** 2")
                lines.append("")
            lines.append(f"w = Worker{i}_0()")
            lines.append(f"print(w.run())")
            cells.append(make_cell("code", "\n".join(lines)))
    return make_notebook(cells)

def main():
    out_dir = sys.argv[1] if len(sys.argv) > 1 else os.path.join(os.path.dirname(__file__), "data")
    os.makedirs(out_dir, exist_ok=True)

    specs = [
        ("small", gen_small),
        ("medium", gen_medium),
        ("large", gen_large),
        ("xlarge", gen_xlarge),
    ]

    for name, gen in specs:
        nb = gen()
        path = os.path.join(out_dir, f"{name}.ipynb")
        with open(path, "w") as f:
            json.dump(nb, f, indent=1)
        size = os.path.getsize(path)
        n_cells = len(nb["cells"])
        print(f"  {name:8s}: {n_cells:4d} cells, {size:>8,d} bytes -> {path}")

if __name__ == "__main__":
    main()
