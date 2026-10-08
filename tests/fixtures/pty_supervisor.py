#!/usr/bin/env python3
"""Hold the controlling session until terminal restoration has been inspected."""
import json
import os
import subprocess
import sys
import termios

report = os.fdopen(int(sys.argv[1]), "w", buffering=1)
before = termios.tcgetattr(0)
child = subprocess.Popen(sys.argv[2:])
report.write(json.dumps({"pid": child.pid}) + "\n")
status = child.wait()
# On macOS the slave can be revoked when its session leader exits. Inspect it
# while this supervisor still owns the session, after the editor has exited.
restored = termios.tcgetattr(0) == before
report.write(json.dumps({"status": status, "restored": restored}) + "\n")
report.close()
