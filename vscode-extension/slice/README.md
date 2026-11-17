# Built-in Slice Files

The Slice files in this directory ship with the extension as its built-in Slice definitions. They are vendored from
[icerpc-slice](https://github.com/icerpc/icerpc-slice), so change them there first and then sync them to this
repository.

[slice.toml](../../slice.toml) names the upstream revision and the files to vendor. To sync the files, run this from
the repository root:

```shell
python build/sync-slice.py
```

To check that the files match upstream, add `--verify`. CI runs this check on every pull request.

The script requires Python 3.11 or later.
