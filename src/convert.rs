use std::collections::{HashMap, HashSet};
use std::io::{self, BufRead, Write};

fn invalid(msg: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg)
}

/// Converts the indented tree format into pid,ppid,name lines.
///
/// Memory use is bounded by the depth of the tree, not by how many
/// processes appear in the input: we only ever keep one pid per open
/// indentation level on `ancestors`, so a file with a million siblings
/// at the same depth costs the same handful of bytes as one with ten.
pub fn flatten<R: BufRead, W: Write>(reader: R, writer: &mut W) -> io::Result<()> {
    let mut ancestors: Vec<(usize, u32)> = Vec::new();
    let mut last_depth: Option<usize> = None;

    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        let indent = line.chars().take_while(|c| *c == ' ').count();
        if indent % 2 != 0 {
            return Err(invalid(format!(
                "indentation must be a multiple of two spaces: {line:?}"
            )));
        }
        let depth = indent / 2;

        match last_depth {
            Some(prev) if depth > prev + 1 => {
                return Err(invalid(format!("indentation skips a level: {line:?}")));
            }
            None if depth != 0 => {
                return Err(invalid(format!("first line must be at depth 0: {line:?}")));
            }
            _ => {}
        }

        let rest = line[indent..].trim_end();
        let (pid_str, name) = rest
            .split_once(' ')
            .ok_or_else(|| invalid(format!("expected \"<pid> <name>\": {line:?}")))?;
        let pid: u32 = pid_str
            .parse()
            .map_err(|_| invalid(format!("not a valid pid: {pid_str:?}")))?;

        while matches!(ancestors.last(), Some((d, _)) if *d >= depth) {
            ancestors.pop();
        }

        let ppid = ancestors.last().map(|&(_, p)| p).unwrap_or(0);
        writeln!(writer, "{pid},{ppid},{name}")?;

        ancestors.push((depth, pid));
        last_depth = Some(depth);
    }

    Ok(())
}

/// Converts pid,ppid,name lines into the indented tree format.
///
/// Unlike `flatten`, this direction cannot stream: a child can appear
/// before its parent in the input, so we have no way to know where a
/// line belongs in the tree until the whole file has been read. This
/// builds a full pid -> children map in memory, proportional to the
/// number of processes in the input.
pub fn nest<R: BufRead, W: Write>(reader: R, writer: &mut W) -> io::Result<()> {
    let mut names: HashMap<u32, String> = HashMap::new();
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut roots: Vec<u32> = Vec::new();

    for line in reader.lines() {
        let line = line?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let mut parts = line.splitn(3, ',');
        let pid: u32 = parts
            .next()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| invalid(format!("bad pid in line: {line:?}")))?;
        let ppid: u32 = parts
            .next()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| invalid(format!("bad ppid in line: {line:?}")))?;
        let name = parts
            .next()
            .ok_or_else(|| invalid(format!("missing name in line: {line:?}")))?
            .to_string();

        names.insert(pid, name);
        if ppid == 0 {
            roots.push(pid);
        } else {
            children.entry(ppid).or_default().push(pid);
        }
    }

    for root in roots {
        let mut path = HashSet::new();
        print_subtree(root, 0, &names, &children, &mut path, writer)?;
    }
    Ok(())
}

// `path` holds the pids on the current root-to-node branch. A flat dump
// is just a list of (pid, ppid) pairs, so nothing stops the input from
// claiming a pid is its own ancestor; without this check that shows up
// as unbounded recursion instead of a readable error.
fn print_subtree(
    pid: u32,
    depth: usize,
    names: &HashMap<u32, String>,
    children: &HashMap<u32, Vec<u32>>,
    path: &mut HashSet<u32>,
    writer: &mut impl Write,
) -> io::Result<()> {
    if !path.insert(pid) {
        return Err(invalid(format!("cycle detected: pid {pid} is its own ancestor")));
    }

    let name = names.get(&pid).map(|s| s.as_str()).unwrap_or("?");
    writeln!(writer, "{}{} {}", "  ".repeat(depth), pid, name)?;
    if let Some(kids) = children.get(&pid) {
        for &kid in kids {
            print_subtree(kid, depth + 1, names, children, path, writer)?;
        }
    }

    path.remove(&pid);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn flatten_str(input: &str) -> io::Result<String> {
        let mut out = Vec::new();
        flatten(Cursor::new(input.as_bytes()), &mut out)?;
        Ok(String::from_utf8(out).unwrap())
    }

    fn nest_str(input: &str) -> io::Result<String> {
        let mut out = Vec::new();
        nest(Cursor::new(input.as_bytes()), &mut out)?;
        Ok(String::from_utf8(out).unwrap())
    }

    #[test]
    fn flatten_basic_tree() {
        let input = "1 init\n  100 sshd\n    150 bash\n  300 cron\n";
        let out = flatten_str(input).unwrap();
        assert_eq!(out, "1,0,init\n100,1,sshd\n150,100,bash\n300,1,cron\n");
    }

    #[test]
    fn flatten_rejects_odd_indentation() {
        let err = flatten_str("1 init\n 100 sshd\n").unwrap_err();
        assert!(err.to_string().contains("multiple of two spaces"));
    }

    #[test]
    fn flatten_rejects_skipped_level() {
        let err = flatten_str("1 init\n    150 bash\n").unwrap_err();
        assert!(err.to_string().contains("skips a level"));
    }

    #[test]
    fn flatten_rejects_indented_first_line() {
        let err = flatten_str("  1 init\n").unwrap_err();
        assert!(err.to_string().contains("first line must be at depth 0"));
    }

    #[test]
    fn flatten_allows_dedent_by_more_than_one_level() {
        // going back up several levels at once is fine, only going
        // *down* more than one level at a time is a malformed jump
        let input = "1 init\n  100 sshd\n    150 bash\n300 cron\n";
        let out = flatten_str(input).unwrap();
        assert_eq!(out, "1,0,init\n100,1,sshd\n150,100,bash\n300,0,cron\n");
    }

    #[test]
    fn nest_basic_tree() {
        let input = "1,0,init\n100,1,sshd\n150,100,bash\n300,1,cron\n";
        let out = nest_str(input).unwrap();
        assert_eq!(out, "1 init\n  100 sshd\n    150 bash\n  300 cron\n");
    }

    #[test]
    fn nest_rejects_self_cycle() {
        let err = nest_str("1,0,init\n5,5,stuck\n").unwrap_err();
        assert!(err.to_string().contains("cycle detected"));
    }

    #[test]
    fn nest_rejects_indirect_cycle() {
        // 1 is a root, but also claims to be a child of 2, which is a
        // child of 1 - following children from 1 would loop forever
        let err = nest_str("1,0,init\n2,1,a\n1,2,x\n").unwrap_err();
        assert!(err.to_string().contains("cycle detected"));
    }
}
