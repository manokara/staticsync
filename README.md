# staticsync

This little tool came out of the following use case: I use an accounting software and I'd like to share its data with other people through cloud sync, however this application (and most Windows apps in general before Windows 10 added automatic link resolving support) doesn't play well with a symlink and considers it as non existant. Having a Windows 7 virtual machine (less overhead), my solution was to write this and let it sync the local version and the cloud version (on a virtual drive) automagically.

staticsync just copies the newer file to the older path. Maybe I'll add delta syncs in the future...

## Usage

    staticsync [OPTIONS]

    OPTIONS:
    -c, --config CONFIG Path to a configuration file. Will use .staticsync.json in your home folder if unspecified.
    -t, --time SECONDS  Delay time between each check
    -s, --size SIZE     Hashing buffer size, in bytes (default: 10 MB, 10485760)
    -n, --once          Only run sync once

## Config format

```json
{
    "files": [
        ["path_a", "path_b"]
    ]
}
```

These paths must be absolute. staticsync will tell you if they're not, if they don't exist, if they're the same or if they're a directory.
