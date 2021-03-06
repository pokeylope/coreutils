#![crate_name = "uu_ls"]

// This file is part of the uutils coreutils package.
//
// (c) Jeremiah Peschka <jeremiah.peschka@gmail.com>
//
// For the full copyright and license information, please view the LICENSE file
// that was distributed with this source code.
//

extern crate getopts;
extern crate pretty_bytes;
use pretty_bytes::converter::convert;

#[macro_use]
extern crate uucore;

extern crate libc;
#[cfg(unix)]
use self::libc::{S_ISUID, S_ISGID, S_ISVTX, S_IRUSR, S_IWUSR, S_IXUSR, S_IRGRP, S_IWGRP, S_IXGRP,
                 S_IROTH, S_IWOTH, S_IXOTH, mode_t, c_char};

use getopts::Options;
use std::fs;
use std::fs::{ReadDir, DirEntry, FileType, Metadata};
use std::ffi::{OsString, CStr};
use std::path::Path;
use std::io::Write;
use std::ptr;

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

#[derive(Copy, Clone, PartialEq)]
enum Mode {
    Help,
    Version,
    List,
}

static NAME: &'static str = "ls";
static VERSION: &'static str = env!("CARGO_PKG_VERSION");

pub fn uumain(args: Vec<String>) -> i32 {
    let mut opts = Options::new();

    opts.optflag("", "help", "display this help and exit");
    opts.optflag("", "version", "output version information and exit");

    opts.optflag("a",
                 "all",
                 "Do not ignore hidden files (files with names that start with '.').");
    opts.optflag("A",
                 "almost-all",
                 "In a directory, do not ignore all file names that start with '.', only ignore \
                  '.' and '..'.");
    opts.optflag("B",
                 "ignore-backups",
                 "Ignore files that end with ~. Equivalent to using `--ignore='*~'` or \
                  `--ignore='.*~'.");
    opts.optflag("d",
                 "directory",
                 "Only list the names of directories, rather than listing directory contents. \
                  This will not follow symbolic links unless one of `--dereference-command-line \
                  (-H)`, `--dereference (-L)`, or `--dereference-command-line-symlink-to-dir` is \
                  specified.");
    opts.optflag("H",
                 "dereference-command-line",
                 "If a command line argument specifies a symbolic link, show information about \
                  the linked file rather than the link itself.");
    opts.optflag("h",
                 "human-readable",
                 "Print human readable file sizes (e.g. 1K 234M 56G).");

    let matches = match opts.parse(&args[1..]) {
        Ok(m) => m,
        Err(e) => {
            show_error!("{}", e);
            panic!()
        }
    };

    let mode = if matches.opt_present("version") {
        Mode::Version
    } else if matches.opt_present("help") {
        Mode::Help
    } else {
        Mode::List
    };

    match mode {
        Mode::Version => version(),
        Mode::Help => help(),
        Mode::List => list(matches),
    }

    0
}

fn version() {
    println!("{} {}", NAME, VERSION);
}

fn help() {
    let msg = format!("{0} {1}\n\n\
                       Usage:  {0} [OPTION]... DIRECTORY \n   \
                          or:  {0} [OPTION]... [FILE]... \n \
                       \n \
                       By default, ls will list the files and contents of any directories on \
                       the command line, expect that it will ignore files and directories \
                       whose names start with '.'. \n\
                       \n",
                      NAME,
                      VERSION);
    println!("{}", msg);
}

fn list(options: getopts::Matches) {
    let locs: Vec<String> = if options.free.is_empty() {
        vec![String::from(".")]
    } else {
        options.free.iter().cloned().collect()
    };

    for loc in locs {
        let p = Path::new(&loc);

        if !p.exists() {
            show_error!("Cannot find path '{}' because it does not exist.", loc);
            panic!();
        }

        if p.is_dir() {
            match fs::read_dir(p) {
                Err(e) => {
                    show_error!("Cannot read directory '{}'. \n Reason: {}", loc, e);
                    panic!();
                }
                Ok(entries) => enter_directory(entries, &options),
            };
        }

        if p.is_file() {
            display_item(Path::new(p), &options)
        }
    }
}

fn max(lhs: usize, rhs: usize) -> usize {
    if lhs > rhs {
      lhs
    } else {
      rhs
    }
}

fn should_cull_dot(file_name: &DirEntry, view_all: bool) -> bool {
    let file_name = file_name.file_name();
    file_name.to_str().map_or(false, |x| {
        if view_all {
            false
        } else if x.chars().next().unwrap() == '.' {
            true
        } else {
            false
        }
    })
}

fn enter_directory(contents: ReadDir, options: &getopts::Matches) {
    let contents = contents.collect::<Vec<_>>();
    let (mut max_links, mut max_size) = (1, 1);
    let culling_dot = options.opt_present("a");
    for entry in &contents {
        let entry = match *entry {
            Err(ref err) => {
                show_error!("{}", err);
                panic!();
            }
            Ok(ref en) => en,
        };
        if should_cull_dot(&entry, culling_dot) {
            continue;
        }
        let (links, size) = display_dir_entry_size(entry, options);
        max_links = max(links, max_links);
        max_size = max(size, max_size);
    }

    for entry in &contents {
        let entry = match *entry {
            Err(ref err) => {
                show_error!("{}", err);
                panic!();
            }
            Ok(ref en) => en,
        };
        if should_cull_dot(&entry, culling_dot) {
            continue;
        }
        // Currently have a DirEntry that we can believe in.
        display_dir_entry(entry, options, max_links, max_size);
    }
}

fn get_metadata(entry: &DirEntry) -> Metadata {
    match entry.metadata() {
        Err(e) => {
            show_error!("Unable to retrieve metadata for {}. \n Error: {}",
                        display_file_name(entry.file_name()),
                        e);
            panic!();
        }
        Ok(md) => md,
    }
}

fn display_dir_entry_size(entry: &DirEntry, options: &getopts::Matches) -> (usize, usize) {
    let md = get_metadata(entry);
    (display_symlink_count(&md).len(), display_file_size(&md, options).len())
}

fn pad_left(string: String, count: usize) -> String {
    if count > string.len() {
        let pad = count - string.len();
        let pad = String::from_utf8(vec![' ' as u8; pad]).unwrap();
        format!("{}{}", pad, string)
    } else {
        string
    }
}

fn display_dir_entry(entry: &DirEntry, options: &getopts::Matches, max_links: usize, max_size: usize) {
    let md = get_metadata(entry);
    println!("{}{} {} {} {} {} {}",
             display_file_type(entry.file_type()),
             display_permissions(&md),
             pad_left(display_symlink_count(&md), max_links),
             display_uname(&md),
             display_group(&md),
             pad_left(display_file_size(&md, options), max_size),
             display_file_name(entry.file_name()));
}

// Currently getpwuid is `linux` target only. If it's broken out into
// a posix-compliant attribute this can be updated...
#[cfg(unix)]
use uucore::c_types::{getpwuid, getgrgid};

// Only used in `display_uname` and `display_group`
#[cfg(unix)]
fn cstr2string(cstr: *const c_char) -> String {
    unsafe { CStr::from_ptr(cstr).to_string_lossy().into_owned() }
}

#[cfg(unix)]
fn display_uname(metadata: &Metadata) -> String {
    let pw = unsafe { getpwuid(metadata.uid()) };
    if !pw.is_null() {
        cstr2string(unsafe { ptr::read(pw).pw_name })
    } else {
        metadata.uid().to_string()
    }
}

#[cfg(unix)]
fn display_group(metadata: &Metadata) -> String {
    let ent = unsafe { getgrgid(metadata.gid()) };
    if !ent.is_null() {
        cstr2string(unsafe { ptr::read(ent).gr_name })
    } else {
        metadata.gid().to_string()
    }
}

#[cfg(not(unix))]
#[allow(unused_variables)]
fn display_uname(metadata: &Metadata) -> String {
    "somebody".to_string()
}

#[cfg(not(unix))]
#[allow(unused_variables)]
fn display_group(metadata: &Metadata) -> String {
    "somegroup".to_string()
}

fn display_file_size(metadata: &Metadata, options: &getopts::Matches) -> String {
    if options.opt_present("human-readable") {
        convert(metadata.len() as f64)
    } else {
        metadata.len().to_string()
    }
}

fn display_file_type(file_type: Result<FileType, std::io::Error>) -> String {
    let file_type = match file_type {
        Err(e) => {
            show_error!("{}", e);
            panic!()
        }
        Ok(ft) => ft,
    };

    if file_type.is_dir() {
        "d".to_string()
    } else if file_type.is_symlink() {
        "l".to_string()
    } else {
        "-".to_string()
    }
}

fn display_file_name(name: OsString) -> String {
    name.to_string_lossy().into_owned()
}

#[cfg(target_family = "windows")]
#[allow(unused_variables)]
fn display_symlink_count(metadata: &Metadata) -> String {
    // Currently not sure of how to get this on Windows, so I'm punting.
    // Git Bash looks like it may do the same thing.
    String::from("1")
}

#[cfg(target_family = "unix")]
fn display_symlink_count(metadata: &Metadata) -> String {
    metadata.nlink().to_string()
}

#[cfg(target_family = "windows")]
#[allow(unused_variables)]
fn display_permissions(metadata: &Metadata) -> String {
    String::from("---------")
}

macro_rules! has {
    ($mode:expr, $perm:expr) => (
        $mode & $perm != 0
    )
}
#[cfg(target_family = "unix")]
fn display_permissions(metadata: &Metadata) -> String {
    let mode = metadata.mode() as mode_t;
    let mut result = String::with_capacity(9);
    result.push(if has!(mode, S_IRUSR) {
        'r'
    } else {
        '-'
    });
    result.push(if has!(mode, S_IWUSR) {
        'w'
    } else {
        '-'
    });
    result.push(if has!(mode, S_ISUID as mode_t) {
        if has!(mode, S_IXUSR) {
            's'
        } else {
            'S'
        }
    } else if has!(mode, S_IXUSR) {
        'x'
    } else {
        '-'
    });

    result.push(if has!(mode, S_IRGRP) {
        'r'
    } else {
        '-'
    });
    result.push(if has!(mode, S_IWGRP) {
        'w'
    } else {
        '-'
    });
    result.push(if has!(mode, S_ISGID as mode_t) {
        if has!(mode, S_IXGRP) {
            's'
        } else {
            'S'
        }
    } else if has!(mode, S_IXGRP) {
        'x'
    } else {
        '-'
    });

    result.push(if has!(mode, S_IROTH) {
        'r'
    } else {
        '-'
    });
    result.push(if has!(mode, S_IWOTH) {
        'w'
    } else {
        '-'
    });
    result.push(if has!(mode, S_ISVTX as mode_t) {
        if has!(mode, S_IXOTH) {
            't'
        } else {
            'T'
        }
    } else if has!(mode, S_IXOTH) {
        'x'
    } else {
        '-'
    });

    result
}

#[allow(unused_variables)]
fn display_item(item: &Path, options: &getopts::Matches) {
    // let fileType = item.file
    // let mut fileMeta = String::new();

    // fileMeta = fileMeta + if item.is_dir() {
    //                             "d"
    // } else if item.sy
    //                         } else {
    //                             "-"
    //                         };



    // println!("{}{}", displayString, item.display());
}
