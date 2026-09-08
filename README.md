# ptreeconv

A small command-line tool that converts between two ways of writing down
a process tree:

**indented** — one process per line, two spaces of indentation per
level of nesting, like `pstree` output:

```
1 init
  100 sshd
    150 bash
      200 vim
  300 cron
```

**flat** — one process per line, `pid,ppid,name`, like a dump of
`ps -eo pid,ppid,comm`:

```
1,0,init
100,1,sshd
150,100,bash
200,150,vim
300,1,cron
```

The flat format is what you actually get from a running system (a
scrape of `/proc`, a `ps` dump, a log line per process). The indented
format is what a human wants to read. This tool goes both ways.

## Usage

```
ptreeconv flatten tree.txt   > flat.csv
ptreeconv nest flat.csv      > tree.txt
cat tree.txt | ptreeconv flatten > flat.csv
```

If no file is given, input is read from stdin. Output always goes to
stdout.

## On memory use

`flatten` (indented -> flat) is a genuine stream: it reads the input
one line at a time and only keeps one pid per currently-open
indentation level. A tree that is a million processes wide but ten
deep costs about ten pids of memory, not a million.

`nest` (flat -> indented) cannot do this. A child's line can appear
before its parent's line anywhere in a flat dump, so there is no way
to know where a process belongs in the tree until every line has been
read. That direction builds a full pid -> children map in memory,
sized to the number of processes in the input. This is a property of
the flat format, not a shortcut taken here.

## Format notes

- Indentation must be spaces, two per level. Tabs are not accepted.
- A line may not jump more than one indentation level deeper than the
  line before it.
- In the flat format, a ppid of `0` marks a root process.
- Process names may contain spaces (in `flatten` output) or commas (in
  `nest` output, since only the first two commas are treated as field
  separators) — whatever the source data has is passed through as-is.

## Building

Standard `cargo build --release`; there are no dependencies to fetch.

## License

MIT, see LICENSE.
