"""Shim: makes `import kuzu` resolve to real_ladybug for Axon compatibility."""
import sys
import real_ladybug
sys.modules["kuzu"] = real_ladybug
