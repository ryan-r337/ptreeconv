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
        print_subtree(root, &names, &children, writer)?;
    }
    Ok(())
}

// A frame per open branch, walked with an explicit stack instead of
// function recursion. A real /proc dump can nest deep enough (a stuck
// fork bomb, a container's control group hierarchy) to blow the call
// stack if each level were a recursive call; this way traversal depth
// only costs a Vec push, not a stack frame.
struct Frame {
    pid: u32,
    depth: usize,
    next_child: usize,
}

fn write_process(
    pid: u32,
    depth: usize,
    names: &HashMap<u32, String>,
    writer: &mut impl Write,
) -> io::Result<()> {
    let name = names.get(&pid).map(|s| s.as_str()).unwrap_or("?");
    writeln!(writer, "{}{} {}", "  ".repeat(depth), pid, name)
}

// `path` holds the pids on the current root-to-node branch. A flat dump
// is just a list of (pid, ppid) pairs, so nothing stops the input from
// claiming a pid is its own ancestor; without this check that shows up
// as an infinite loop instead of a readable error.
fn print_subtree(
    root: u32,
    names: &HashMap<u32, String>,
    children: &HashMap<u32, Vec<u32>>,
    writer: &mut impl Write,
) -> io::Result<()> {
    let mut path = HashSet::new();
    path.insert(root);
    write_process(root, 0, names, writer)?;

    let mut stack = vec![Frame { pid: root, depth: 0, next_child: 0 }];

    while let Some(frame) = stack.last_mut() {
        let next = children
            .get(&frame.pid)
            .and_then(|kids| kids.get(frame.next_child));

        match next {
            Some(&kid) => {
                frame.next_child += 1;
                if !path.insert(kid) {
                    return Err(invalid(format!(
                        "cycle detected: pid {kid} is its own ancestor"
                    )));
                }
                let depth = frame.depth + 1;
                write_process(kid, depth, names, writer)?;
                stack.push(Frame { pid: kid, depth, next_child: 0 });
            }
            None => {
                path.remove(&frame.pid);
                stack.pop();
            }
        }
    }

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
    fn nest_handles_a_deep_chain_iteratively() {
        // each pid's parent is the one before it, so this is a single
        // branch a thousand levels deep - the kind of input a recursive
        // walk of the tree could blow the stack on.
        let depth = 1000;
        let mut input = String::new();
        input.push_str("1,0,p1\n");
        for pid in 2..=depth {
            input.push_str(&format!("{pid},{},p{pid}\n", pid - 1));
        }

        let out = nest_str(&input).unwrap();
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), depth as usize);
        assert_eq!(lines[0], "1 p1");
        assert_eq!(lines[1], "  2 p2");
        let last = format!("{}{} p{}", "  ".repeat(depth as usize - 1), depth, depth);
        assert_eq!(lines[depth as usize - 1], last);
    }

    #[test]
    fn nest_rejects_indirect_cycle() {
        // 1 is a root, but also claims to be a child of 2, which is a
        // child of 1 - following children from 1 would loop forever
        let err = nest_str("1,0,init\n2,1,a\n1,2,x\n").unwrap_err();
        assert!(err.to_string().contains("cycle detected"));
    }
}
