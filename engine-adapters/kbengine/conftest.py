"""Make the `bridge` package importable when running pytest from the adapter
root. The engine process would have the package on sys.path via its script
search path; tests reach it through this bootstrap."""
