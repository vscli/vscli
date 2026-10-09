#!/usr/bin/env python3
"""Deterministic process lifecycle wrapper around the signature LSP fixture."""
import os
from pathlib import Path
import runpy
import sys
import threading
import time

marker = Path(sys.argv[1])
with marker.open("a") as stream:
    stream.write(f"{os.getpid()}\n")

def crash():
    while not marker.with_suffix(".crash").exists():
        time.sleep(0.005)
    os._exit(3)

threading.Thread(target=crash, daemon=True).start()
runpy.run_path(str(Path(__file__).with_name("signature_server.py")), run_name="__main__")
