#!/usr/bin/env python3
import re
import sys

if len(sys.argv) < 2:
    print("usage: extract_flag.py <file>")
    sys.exit(1)

with open(sys.argv[1], "r", errors="ignore") as f:
    text = f.read()

for hit in re.findall(r"[A-Za-z0-9_\-]{2,16}\{[^\n\r\}]{1,180}\}", text):
    print(hit)
