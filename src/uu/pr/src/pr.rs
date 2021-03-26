#![crate_name = "uu_pr"]

// This file is part of the uutils coreutils package.
//
// For the full copyright and license information, please view the LICENSE file
// that was distributed with this source code.
//

#[macro_use]
extern crate quick_error;

use chrono::offset::Local;
use chrono::DateTime;
use getopts::{HasArg, Occur};
use getopts::{Matches, Options};
use itertools::structs::KMergeBy;
use itertools::{GroupBy, Itertools};
use quick_error::ResultExt;
use regex::Regex;
use std::convert::From;
use std::fs::{metadata, File, Metadata};
use std::io::{stdin, stdout, BufRead, BufReader, Lines, Read, Stdin, Stdout, Write};
use std::iter::{FlatMap, Map};
use std::num::ParseIntError;
#[cfg(unix)]
use std::os::unix::fs::FileTypeExt;
use std::vec::Vec;

type IOError = std::io::Error;

static NAME: &str = "pr";
static VERSION: &str = env!("CARGO_PKG_VERSION");
static TAB: char = '\t';
static LINES_PER_PAGE: usize = 66;
static LINES_PER_PAGE_FOR_FORM_FEED: usize = 63;
static HEADER_LINES_PER_PAGE: usize = 5;
static TRAILER_LINES_PER_PAGE: usize = 5;
static STRING_HEADER_OPTION: &str = "h";
static DOUBLE_SPACE_OPTION: &str = "d";
static NUMBERING_MODE_OPTION: &str = "n";
static FIRST_LINE_NUMBER_OPTION: &str = "N";
static PAGE_RANGE_OPTION: &str = "pages";
static NO_HEADER_TRAILER_OPTION: &str = "t";
static PAGE_LENGTH_OPTION: &str = "l";
static SUPPRESS_PRINTING_ERROR: &str = "r";
static FORM_FEED_OPTION: &str = "F";
static FORM_FEED_OPTION_SMALL: &str = "f";
static COLUMN_WIDTH_OPTION: &str = "w";
static PAGE_WIDTH_OPTION: &str = "W";
static ACROSS_OPTION: &str = "a";
static COLUMN_OPTION: &str = "column";
static COLUMN_CHAR_SEPARATOR_OPTION: &str = "s";
static COLUMN_STRING_SEPARATOR_OPTION: &str = "S";
static MERGE_FILES_PRINT: &str = "m";
static OFFSET_SPACES_OPTION: &str = "o";
static JOIN_LINES_OPTION: &str = "J";
static FILE_STDIN: &str = "-";
static READ_BUFFER_SIZE: usize = 1024 * 64;
static DEFAULT_COLUMN_WIDTH: usize = 72;
static DEFAULT_COLUMN_WIDTH_WITH_S_OPTION: usize = 512;
static DEFAULT_COLUMN_SEPARATOR: &char = &TAB;
static FF: u8 = 0x0C as u8;

struct OutputOptions {
    /// Line numbering mode
    number: Option<NumberingMode>,
    header: String,
    double_space: bool,
    line_separator: String,
    content_line_separator: String,
    last_modified_time: String,
    start_page: usize,
    end_page: Option<usize>,
    display_header_and_trailer: bool,
    content_lines_per_page: usize,
    page_separator_char: String,
    column_mode_options: Option<ColumnModeOptions>,
    merge_files_print: Option<usize>,
    offset_spaces: String,
    form_feed_used: bool,
    join_lines: bool,
    col_sep_for_printing: String,
    line_width: Option<usize>,
}

struct FileLine {
    file_id: usize,
    line_number: usize,
    page_number: usize,
    group_key: usize,
    line_content: Result<String, IOError>,
    form_feeds_after: usize,
}

impl AsRef<FileLine> for FileLine {
    fn as_ref(&self) -> &FileLine {
        self
    }
}

struct ColumnModeOptions {
    width: usize,
    columns: usize,
    column_separator: String,
    across_mode: bool,
}

impl AsRef<OutputOptions> for OutputOptions {
    fn as_ref(&self) -> &OutputOptions {
        self
    }
}

struct NumberingMode {
    /// Line numbering mode
    width: usize,
    separator: String,
    first_number: usize,
}

impl Default for NumberingMode {
    fn default() -> NumberingMode {
        NumberingMode {
            width: 5,
            separator: TAB.to_string(),
            first_number: 1,
        }
    }
}

impl Default for FileLine {
    fn default() -> FileLine {
        FileLine {
            file_id: 0,
            line_number: 0,
            page_number: 0,
            group_key: 0,
            line_content: Ok(String::new()),
            form_feeds_after: 0,
        }
    }
}

impl From<IOError> for PrError {
    fn from(err: IOError) -> Self {
        PrError::EncounteredErrors(err.to_string())
    }
}

quick_error! {
    #[derive(Debug)]
    enum PrError {
        Input(err: IOError, path: String) {
            context(path: &'a str, err: IOError) -> (err, path.to_owned())
            display("pr: Reading from input {0} gave error", path)
            cause(err)
        }

        UnknownFiletype(path: String) {
            display("pr: {0}: unknown filetype", path)
        }

        EncounteredErrors(msg: String) {
            display("pr: {0}", msg)
        }

        IsDirectory(path: String) {
            display("pr: {0}: Is a directory", path)
        }

        IsSocket(path: String) {
            display("pr: cannot open {}, Operation not supported on socket", path)
        }

        NotExists(path: String) {
            display("pr: cannot open {}, No such file or directory", path)
        }
    }
}

pub fn uumain(args: impl uucore::Args) -> i32 {
    let args = args.collect_str();
    let mut opts = getopts::Options::new();

    opts.opt(
        "",
        PAGE_RANGE_OPTION,
        "Begin and stop printing with page FIRST_PAGE[:LAST_PAGE]",
        "FIRST_PAGE[:LAST_PAGE]",
        HasArg::Yes,
        Occur::Optional,
    );

    opts.opt(
        STRING_HEADER_OPTION,
        "header",
        "Use the string header to replace the file name \
         in the header line.",
        "STRING",
        HasArg::Yes,
        Occur::Optional,
    );

    opts.opt(
        DOUBLE_SPACE_OPTION,
        "double-space",
        "Produce output that is double spaced. An extra <newline> character is output following every <newline>
           found in the input.",
        "",
        HasArg::No,
        Occur::Optional,
    );

    opts.opt(
        NUMBERING_MODE_OPTION,
        "number-lines",
        "Provide width digit line numbering.  The default for width, if not specified, is 5.  The number occupies
           the first width column positions of each text column or each line of -m output.  If char (any nondigit
           character) is given, it is appended to the line number to separate it from whatever follows.  The default
           for char is a <tab>.  Line numbers longer than width columns are truncated.",
        "[char][width]",
        HasArg::Maybe,
        Occur::Optional,
    );

    opts.opt(
        FIRST_LINE_NUMBER_OPTION,
        "first-line-number",
        "start counting with NUMBER at 1st line of first page printed",
        "NUMBER",
        HasArg::Yes,
        Occur::Optional,
    );

    opts.opt(
        NO_HEADER_TRAILER_OPTION,
        "omit-header",
        "Write neither the five-line identifying header nor the five-line trailer usually supplied for  each  page.  Quit
              writing after the last line of each file without spacing to the end of the page.",
        "",
        HasArg::No,
        Occur::Optional,
    );

    opts.opt(
        PAGE_LENGTH_OPTION,
        "length",
        "Override the 66-line default (default number of lines of text 56, and with -F 63) and reset the page length to lines.  If lines is not greater than the sum  of  both
              the  header  and trailer depths (in lines), the pr utility shall suppress both the header and trailer, as if the
              -t option were in effect. ",
        "lines",
        HasArg::Yes,
        Occur::Optional,
    );

    opts.opt(
        SUPPRESS_PRINTING_ERROR,
        "no-file-warnings",
        "omit warning when a file cannot be opened",
        "",
        HasArg::No,
        Occur::Optional,
    );

    opts.opt(
        FORM_FEED_OPTION,
        "form-feed",
        "Use a <form-feed> for new pages, instead of the default behavior that uses a sequence of <newline>s.",
        "",
        HasArg::No,
        Occur::Optional,
    );
    opts.opt(
        FORM_FEED_OPTION_SMALL,
        "form-feed",
        "Same as -F but pause before beginning the first page if standard output is a
           terminal.",
        "",
        HasArg::No,
        Occur::Optional,
    );

    opts.opt(
        "",
        COLUMN_OPTION,
        "Produce multi-column output that is arranged in column columns (the default shall be 1) and is written down each
              column  in  the order in which the text is received from the input file. This option should not be used with -m.
              The options -e and -i shall be assumed for multiple text-column output.  Whether or not text  columns  are  pro‐
              duced  with  identical  vertical  lengths is unspecified, but a text column shall never exceed the length of the
              page (see the -l option). When used with -t, use the minimum number of lines to write the output.",
        "[column]",
        HasArg::Yes,
        Occur::Optional,
    );

    opts.opt(
        COLUMN_WIDTH_OPTION,
        "width",
        "Set  the  width  of the line to width column positions for multiple text-column output only. If the -w option is
              not specified and the -s option is not specified, the default width shall be 72. If the -w option is not  speci‐
              fied and the -s option is specified, the default width shall be 512.",
        "[width]",
        HasArg::Yes,
        Occur::Optional,
    );

    opts.opt(
        PAGE_WIDTH_OPTION,
        "page-width",
        "set page width to PAGE_WIDTH (72) characters always,
        truncate lines, except -J option is set, no interference
        with -S or -s",
        "[width]",
        HasArg::Yes,
        Occur::Optional,
    );

    opts.opt(
        ACROSS_OPTION,
        "across",
        "Modify the effect of the - column option so that the columns are filled across the page in a  round-robin  order
              (for example, when column is 2, the first input line heads column 1, the second heads column 2, the third is the
              second line in column 1, and so on).",
        "",
        HasArg::No,
        Occur::Optional,
    );

    opts.opt(
        COLUMN_CHAR_SEPARATOR_OPTION,
        "separator",
        "Separate text columns by the single character char instead of by the appropriate number of <space>s
           (default for char is the <tab> character).",
        "char",
        HasArg::Yes,
        Occur::Optional,
    );

    opts.opt(
        COLUMN_STRING_SEPARATOR_OPTION,
        "sep-string",
        "separate columns by STRING,
        without -S: Default separator <TAB> with -J and <space>
        otherwise (same as -S\" \"), no effect on column options",
        "string",
        HasArg::Yes,
        Occur::Optional,
    );

    opts.opt(
        MERGE_FILES_PRINT,
        "merge",
        "Merge files. Standard output shall be formatted so the pr utility writes one line from each file specified by  a
              file  operand, side by side into text columns of equal fixed widths, in terms of the number of column positions.
              Implementations shall support merging of at least nine file operands.",
        "",
        HasArg::No,
        Occur::Optional,
    );

    opts.opt(
        OFFSET_SPACES_OPTION,
        "indent",
        "Each  line of output shall be preceded by offset <space>s. If the -o option is not specified, the default offset
              shall be zero. The space taken is in addition to the output line width (see the -w option below).",
        "offset",
        HasArg::Yes,
        Occur::Optional,
    );

    opts.opt(
        JOIN_LINES_OPTION,
        "join-lines",
        "merge full lines, turns off -W line truncation, no column
    alignment, --sep-string[=STRING] sets separators",
        "offset",
        HasArg::No,
        Occur::Optional,
    );

    opts.optflag("", "help", "display this help and exit");
    opts.optflag("V", "version", "output version information and exit");

    let opt_args: Vec<String> = recreate_arguments(&args);

    let matches = match opts.parse(&opt_args[1..]) {
        Ok(m) => m,
        Err(e) => panic!("Invalid options\n{}", e),
    };

    if matches.opt_present("version") {
        println!("{} {}", NAME, VERSION);
        return 0;
    }

    let mut files: Vec<String> = matches.free.clone();
    if files.is_empty() {
        //For stdin
        files.insert(0, FILE_STDIN.to_owned());
    }

    if matches.opt_present("help") {
        return print_usage(&mut opts, &matches);
    }

    let file_groups: Vec<Vec<String>> = if matches.opt_present(MERGE_FILES_PRINT) {
        vec![files]
    } else {
        files.into_iter().map(|i| vec![i]).collect()
    };

    for file_group in file_groups {
        let result_options: Result<OutputOptions, PrError> =
            build_options(&matches, &file_group, args.join(" "));

        if result_options.is_err() {
            print_error(&matches, result_options.err().unwrap());
            return 1;
        }

        let options: &OutputOptions = &result_options.unwrap();

        let cmd_result: Result<i32, PrError> = if file_group.len() == 1 {
            pr(&file_group.get(0).unwrap(), options)
        } else {
            mpr(&file_group, options)
        };

        let status: i32 = match cmd_result {
            Err(error) => {
                print_error(&matches, error);
                1
            }
            _ => 0,
        };
        if status != 0 {
            return status;
        }
    }

    0
}

/// Returns re-written arguments which are passed to the program.
/// Removes -column and +page option as getopts cannot parse things like -3 etc
/// # Arguments
/// * `args` - Command line arguments
fn recreate_arguments(args: &Vec<String>) -> Vec<String> {
    let column_page_option = Regex::new(r"^[-+]\d+.*").unwrap();
    let num_regex: Regex = Regex::new(r"(.\d+)|(\d+)|^[^-]$").unwrap();
    //let a_file: Regex = Regex::new(r"^[^-+].*").unwrap();
    let n_regex: Regex = Regex::new(r"^-n\s*$").unwrap();
    let mut arguments = args.clone();
    let num_option: Option<(usize, &String)> =
        args.iter().find_position(|x| n_regex.is_match(x.trim()));
    if num_option.is_some() {
        let (pos, _value) = num_option.unwrap();
        let num_val_opt = args.get(pos + 1);
        if num_val_opt.is_some() && !num_regex.is_match(num_val_opt.unwrap()) {
            let could_be_file = arguments.remove(pos + 1);
            arguments.insert(pos + 1, format!("{}", NumberingMode::default().width));
            // FIXME: the following line replaces the block below that had the same
            // code for both conditional branches. Figure this out.
            arguments.insert(pos + 2, could_be_file);
            // if a_file.is_match(could_be_file.trim().as_ref()) {
            //     arguments.insert(pos + 2, could_be_file);
            // } else {
            //     arguments.insert(pos + 2, could_be_file);
            // }
        }
    }

    arguments
        .into_iter()
        .filter(|i| !column_page_option.is_match(i))
        .collect()
}

fn print_error(matches: &Matches, err: PrError) {
    if !matches.opt_present(SUPPRESS_PRINTING_ERROR) {
        eprintln!("{}", err);
    }
}

fn print_usage(opts: &mut Options, matches: &Matches) -> i32 {
    println!("{} {} -- print files", NAME, VERSION);
    println!();
    println!(
        "Usage: {} [+page] [-column] [-adFfmprt] [[-e] [char] [gap]]
        [-L locale] [-h header] [[-i] [char] [gap]]
        [-l lines] [-o offset] [[-s] [char]] [[-n] [char]
        [width]] [-w width] [-] [file ...].",
        NAME
    );
    println!();
    let usage: &str = "The pr utility is a printing and pagination filter
     for text files.  When multiple input files are spec-
     ified, each is read, formatted, and written to stan-
     dard output.  By default, the input is separated
     into 66-line pages, each with

     o   A 5-line header with the page number, date,
         time, and the pathname of the file.

     o   A 5-line trailer consisting of blank lines.

     If standard output is associated with a terminal,
     diagnostic messages are suppressed until the pr
     utility has completed processing.

     When multiple column output is specified, text col-
     umns are of equal width.  By default text columns
     are separated by at least one <blank>.  Input lines
     that do not fit into a text column are truncated.
     Lines are not truncated under single column output.";
    println!("{}", opts.usage(usage));
    println!("    +page \t\tBegin output at page number page of the formatted input.");
    println!(
        "    -column \t\tProduce multi-column output. Refer --{}",
        COLUMN_OPTION
    );
    if matches.free.is_empty() {
        return 1;
    }

    0
}

fn parse_usize(matches: &Matches, opt: &str) -> Option<Result<usize, PrError>> {
    let from_parse_error_to_pr_error = |value_to_parse: (String, String)| {
        let i = value_to_parse.0;
        let option = value_to_parse.1;
        i.parse::<usize>().map_err(|_e| {
            PrError::EncounteredErrors(format!("invalid {} argument '{}'", option, i))
        })
    };
    matches
        .opt_str(opt)
        .map(|i| (i, format!("-{}", opt)))
        .map(from_parse_error_to_pr_error)
}

fn build_options(
    matches: &Matches,
    paths: &Vec<String>,
    free_args: String,
) -> Result<OutputOptions, PrError> {
    let form_feed_used =
        matches.opt_present(FORM_FEED_OPTION) || matches.opt_present(FORM_FEED_OPTION_SMALL);

    let is_merge_mode: bool = matches.opt_present(MERGE_FILES_PRINT);

    if is_merge_mode && matches.opt_present(COLUMN_OPTION) {
        let err_msg: String =
            String::from("cannot specify number of columns when printing in parallel");
        return Err(PrError::EncounteredErrors(err_msg));
    }

    if is_merge_mode && matches.opt_present(ACROSS_OPTION) {
        let err_msg: String =
            String::from("cannot specify both printing across and printing in parallel");
        return Err(PrError::EncounteredErrors(err_msg));
    }

    let merge_files_print: Option<usize> = if matches.opt_present(MERGE_FILES_PRINT) {
        Some(paths.len())
    } else {
        None
    };

    let header: String = matches
        .opt_str(STRING_HEADER_OPTION)
        .unwrap_or(if is_merge_mode {
            String::new()
        } else {
            if paths[0] == FILE_STDIN {
                String::new()
            } else {
                paths[0].to_string()
            }
        });

    let default_first_number: usize = NumberingMode::default().first_number;
    let first_number: usize =
        parse_usize(matches, FIRST_LINE_NUMBER_OPTION).unwrap_or(Ok(default_first_number))?;

    let number: Option<NumberingMode> = matches
        .opt_str(NUMBERING_MODE_OPTION)
        .map(|i| {
            let parse_result: Result<usize, ParseIntError> = i.parse::<usize>();

            let separator: String = if parse_result.is_err() {
                i[0..1].to_string()
            } else {
                NumberingMode::default().separator
            };

            let width: usize = if parse_result.is_err() {
                i[1..]
                    .parse::<usize>()
                    .unwrap_or(NumberingMode::default().width)
            } else {
                parse_result.unwrap()
            };

            NumberingMode {
                width,
                separator,
                first_number,
            }
        })
        .or_else(|| {
            if matches.opt_present(NUMBERING_MODE_OPTION) {
                return Some(NumberingMode::default());
            }

            None
        });

    let double_space: bool = matches.opt_present(DOUBLE_SPACE_OPTION);

    let content_line_separator: String = if double_space {
        "\n".repeat(2)
    } else {
        "\n".to_string()
    };

    let line_separator: String = "\n".to_string();

    let last_modified_time: String = if is_merge_mode || paths[0].eq(FILE_STDIN) {
        let datetime: DateTime<Local> = Local::now();
        datetime.format("%b %d %H:%M %Y").to_string()
    } else {
        file_last_modified_time(paths.get(0).unwrap())
    };

    // +page option is less priority than --pages
    let page_plus_re = Regex::new(r"\s*\+(\d+:*\d*)\s*").unwrap();
    let start_page_in_plus_option: usize = match page_plus_re.captures(&free_args).map(|i| {
        let unparsed_num = i.get(1).unwrap().as_str().trim();
        let x: Vec<&str> = unparsed_num.split(':').collect();
        x[0].to_string().parse::<usize>().map_err(|_e| {
            PrError::EncounteredErrors(format!("invalid {} argument '{}'", "+", unparsed_num))
        })
    }) {
        Some(res) => res?,
        _ => 1,
    };

    let end_page_in_plus_option: Option<usize> = match page_plus_re
        .captures(&free_args)
        .map(|i| i.get(1).unwrap().as_str().trim())
        .filter(|i| i.contains(':'))
        .map(|unparsed_num| {
            let x: Vec<&str> = unparsed_num.split(':').collect();
            x[1].to_string().parse::<usize>().map_err(|_e| {
                PrError::EncounteredErrors(format!("invalid {} argument '{}'", "+", unparsed_num))
            })
        }) {
        Some(res) => Some(res?),
        _ => None,
    };

    let invalid_pages_map = |i: String| {
        let unparsed_value: String = matches.opt_str(PAGE_RANGE_OPTION).unwrap();
        i.parse::<usize>().map_err(|_e| {
            PrError::EncounteredErrors(format!("invalid --pages argument '{}'", unparsed_value))
        })
    };

    let start_page: usize = match matches
        .opt_str(PAGE_RANGE_OPTION)
        .map(|i| {
            let x: Vec<&str> = i.split(':').collect();
            x[0].to_string()
        })
        .map(invalid_pages_map)
    {
        Some(res) => res?,
        _ => start_page_in_plus_option,
    };

    let end_page: Option<usize> = match matches
        .opt_str(PAGE_RANGE_OPTION)
        .filter(|i: &String| i.contains(':'))
        .map(|i: String| {
            let x: Vec<&str> = i.split(':').collect();
            x[1].to_string()
        })
        .map(invalid_pages_map)
    {
        Some(res) => Some(res?),
        _ => end_page_in_plus_option,
    };

    if end_page.is_some() && start_page > end_page.unwrap() {
        return Err(PrError::EncounteredErrors(format!(
            "invalid --pages argument '{}:{}'",
            start_page,
            end_page.unwrap()
        )));
    }

    let default_lines_per_page = if form_feed_used {
        LINES_PER_PAGE_FOR_FORM_FEED
    } else {
        LINES_PER_PAGE
    };

    let page_length: usize =
        parse_usize(matches, PAGE_LENGTH_OPTION).unwrap_or(Ok(default_lines_per_page))?;

    let page_length_le_ht: bool = page_length < (HEADER_LINES_PER_PAGE + TRAILER_LINES_PER_PAGE);

    let display_header_and_trailer: bool =
        !(page_length_le_ht) && !matches.opt_present(NO_HEADER_TRAILER_OPTION);

    let content_lines_per_page: usize = if page_length_le_ht {
        page_length
    } else {
        page_length - (HEADER_LINES_PER_PAGE + TRAILER_LINES_PER_PAGE)
    };

    let page_separator_char: String = if matches.opt_present(FORM_FEED_OPTION) {
        let bytes = vec![FF];
        String::from_utf8(bytes).unwrap()
    } else {
        "\n".to_string()
    };

    let across_mode: bool = matches.opt_present(ACROSS_OPTION);

    let column_separator: String = match matches.opt_str(COLUMN_STRING_SEPARATOR_OPTION) {
        Some(x) => Some(x),
        None => matches.opt_str(COLUMN_CHAR_SEPARATOR_OPTION),
    }
    .unwrap_or(DEFAULT_COLUMN_SEPARATOR.to_string());

    let default_column_width = if matches.opt_present(COLUMN_WIDTH_OPTION)
        && matches.opt_present(COLUMN_CHAR_SEPARATOR_OPTION)
    {
        DEFAULT_COLUMN_WIDTH_WITH_S_OPTION
    } else {
        DEFAULT_COLUMN_WIDTH
    };

    let column_width: usize =
        parse_usize(matches, COLUMN_WIDTH_OPTION).unwrap_or(Ok(default_column_width))?;

    let page_width: Option<usize> = if matches.opt_present(JOIN_LINES_OPTION) {
        None
    } else {
        match parse_usize(matches, PAGE_WIDTH_OPTION) {
            Some(res) => Some(res?),
            None => None,
        }
    };

    let re_col = Regex::new(r"\s*-(\d+)\s*").unwrap();

    let start_column_option: Option<usize> = match re_col.captures(&free_args).map(|i| {
        let unparsed_num = i.get(1).unwrap().as_str().trim();
        unparsed_num.parse::<usize>().map_err(|_e| {
            PrError::EncounteredErrors(format!("invalid {} argument '{}'", "-", unparsed_num))
        })
    }) {
        Some(res) => Some(res?),
        _ => None,
    };

    // --column has more priority than -column

    let column_option_value: Option<usize> = match parse_usize(matches, COLUMN_OPTION) {
        Some(res) => Some(res?),
        _ => start_column_option,
    };

    let column_mode_options: Option<ColumnModeOptions> = match column_option_value {
        Some(columns) => Some(ColumnModeOptions {
            columns,
            width: column_width,
            column_separator,
            across_mode,
        }),
        _ => None,
    };

    let offset_spaces: String =
        " ".repeat(parse_usize(matches, OFFSET_SPACES_OPTION).unwrap_or(Ok(0))?);
    let join_lines: bool = matches.opt_present(JOIN_LINES_OPTION);

    let col_sep_for_printing = column_mode_options
        .as_ref()
        .map(|i| i.column_separator.clone())
        .unwrap_or(
            merge_files_print
                .map(|_k| DEFAULT_COLUMN_SEPARATOR.to_string())
                .unwrap_or(String::new()),
        );

    let columns_to_print =
        merge_files_print.unwrap_or(column_mode_options.as_ref().map(|i| i.columns).unwrap_or(1));

    let line_width: Option<usize> = if join_lines {
        None
    } else if columns_to_print > 1 {
        Some(
            column_mode_options
                .as_ref()
                .map(|i| i.width)
                .unwrap_or(DEFAULT_COLUMN_WIDTH),
        )
    } else {
        page_width
    };

    Ok(OutputOptions {
        number,
        header,
        double_space,
        line_separator,
        content_line_separator,
        last_modified_time,
        start_page,
        end_page,
        display_header_and_trailer,
        content_lines_per_page,
        page_separator_char,
        column_mode_options,
        merge_files_print,
        offset_spaces,
        form_feed_used,
        join_lines,
        col_sep_for_printing,
        line_width,
    })
}

fn open(path: &str) -> Result<Box<dyn Read>, PrError> {
    if path == FILE_STDIN {
        let stdin: Stdin = stdin();
        return Ok(Box::new(stdin) as Box<dyn Read>);
    }

    metadata(path)
        .map(|i: Metadata| {
            let path_string = path.to_string();
            match i.file_type() {
                #[cfg(unix)]
                ft if ft.is_block_device() => Err(PrError::UnknownFiletype(path_string)),
                #[cfg(unix)]
                ft if ft.is_char_device() => Err(PrError::UnknownFiletype(path_string)),
                #[cfg(unix)]
                ft if ft.is_fifo() => Err(PrError::UnknownFiletype(path_string)),
                #[cfg(unix)]
                ft if ft.is_socket() => Err(PrError::IsSocket(path_string)),
                ft if ft.is_dir() => Err(PrError::IsDirectory(path_string)),
                ft if ft.is_file() || ft.is_symlink() => {
                    Ok(Box::new(File::open(path).context(path)?) as Box<dyn Read>)
                }
                _ => Err(PrError::UnknownFiletype(path_string)),
            }
        })
        .unwrap_or(Err(PrError::NotExists(path.to_string())))
}

fn split_lines_if_form_feed(file_content: Result<String, IOError>) -> Vec<FileLine> {
    file_content
        .map(|content| {
            let mut lines: Vec<FileLine> = Vec::new();
            let mut f_occurred: usize = 0;
            let mut chunk: Vec<u8> = Vec::new();
            for byte in content.as_bytes() {
                if byte == &FF {
                    f_occurred += 1;
                } else {
                    if f_occurred != 0 {
                        // First time byte occurred in the scan
                        lines.push(FileLine {
                            line_content: Ok(String::from_utf8(chunk.clone()).unwrap()),
                            form_feeds_after: f_occurred,
                            ..FileLine::default()
                        });
                        chunk.clear();
                    }
                    chunk.push(*byte);
                    f_occurred = 0;
                }
            }

            lines.push(FileLine {
                line_content: Ok(String::from_utf8(chunk).unwrap()),
                form_feeds_after: f_occurred,
                ..FileLine::default()
            });

            lines
        })
        .unwrap_or_else(|e| {
            vec![FileLine {
                line_content: Err(e),
                ..FileLine::default()
            }]
        })
}

fn pr(path: &str, options: &OutputOptions) -> Result<i32, PrError> {
    let lines: Lines<BufReader<Box<dyn Read>>> =
        BufReader::with_capacity(READ_BUFFER_SIZE, open(path)?).lines();

    let pages: Box<dyn Iterator<Item = (usize, Vec<FileLine>)>> =
        read_stream_and_create_pages(options, lines, 0);

    for page_with_page_number in pages {
        let page_number = page_with_page_number.0 + 1;
        let page = page_with_page_number.1;
        print_page(&page, options, page_number)?;
    }

    Ok(0)
}

fn read_stream_and_create_pages(
    options: &OutputOptions,
    lines: Lines<BufReader<Box<dyn Read>>>,
    file_id: usize,
) -> Box<dyn Iterator<Item = (usize, Vec<FileLine>)>> {
    let start_page: usize = options.start_page;
    let start_line_number: usize = get_start_line_number(options);
    let last_page: Option<usize> = options.end_page;
    let lines_needed_per_page: usize = lines_to_read_for_page(options);

    Box::new(
        lines
            .map(split_lines_if_form_feed)
            .flatten()
            .enumerate()
            .map(move |i: (usize, FileLine)| FileLine {
                line_number: i.0 + start_line_number,
                file_id,
                ..i.1
            }) // Add line number and file_id
            .batching(move |it| {
                let mut first_page: Vec<FileLine> = Vec::new();
                let mut page_with_lines: Vec<Vec<FileLine>> = Vec::new();
                for line in it {
                    let form_feeds_after = line.form_feeds_after;
                    first_page.push(line);

                    if form_feeds_after > 1 {
                        // insert empty pages
                        page_with_lines.push(first_page);
                        for _i in 1..form_feeds_after {
                            page_with_lines.push(vec![]);
                        }
                        return Some(page_with_lines);
                    }

                    if first_page.len() == lines_needed_per_page || form_feeds_after == 1 {
                        break;
                    }
                }

                if first_page.is_empty() {
                    return None;
                }
                page_with_lines.push(first_page);
                Some(page_with_lines)
            }) // Create set of pages as form feeds could lead to empty pages
            .flatten() // Flatten to pages from page sets
            .enumerate() // Assign page number
            .skip_while(move |x: &(usize, Vec<FileLine>)| {
                // Skip the not needed pages
                let current_page = x.0 + 1;

                current_page < start_page
            })
            .take_while(move |x: &(usize, Vec<FileLine>)| {
                // Take only the required pages
                let current_page = x.0 + 1;

                current_page >= start_page
                    && (last_page.is_none() || current_page <= last_page.unwrap())
            }),
    )
}

fn mpr(paths: &Vec<String>, options: &OutputOptions) -> Result<i32, PrError> {
    let nfiles = paths.len();

    // Check if files exists
    for path in paths {
        open(path)?;
    }

    let file_line_groups: GroupBy<
        usize,
        KMergeBy<FlatMap<Map<Box<dyn Iterator<Item = (usize, Vec<FileLine>)>>, _>, _, _>, _>,
        _,
    > = paths
        .iter()
        .enumerate()
        .map(|indexed_path: (usize, &String)| {
            let lines =
                BufReader::with_capacity(READ_BUFFER_SIZE, open(indexed_path.1).unwrap()).lines();

            read_stream_and_create_pages(options, lines, indexed_path.0)
                .map(move |x: (usize, Vec<FileLine>)| {
                    let file_line = x.1;
                    let page_number = x.0 + 1;
                    file_line
                        .into_iter()
                        .map(|fl| FileLine {
                            page_number,
                            group_key: page_number * nfiles + fl.file_id,
                            ..fl
                        })
                        .collect()
                })
                .flat_map(|x: Vec<FileLine>| x)
        })
        .kmerge_by(|a: &FileLine, b: &FileLine| {
            if a.group_key == b.group_key {
                a.line_number < b.line_number
            } else {
                a.group_key < b.group_key
            }
        })
        .group_by(|file_line: &FileLine| file_line.group_key);

    let start_page: usize = options.start_page;
    let mut lines: Vec<FileLine> = Vec::new();
    let mut page_counter = start_page;

    for (_key, file_line_group) in file_line_groups.into_iter() {
        for file_line in file_line_group {
            if file_line.line_content.is_err() {
                return Err(file_line.line_content.unwrap_err().into());
            }
            let new_page_number = file_line.page_number;
            if page_counter != new_page_number {
                print_page(&lines, options, page_counter)?;
                lines = Vec::new();
                page_counter = new_page_number;
            }
            lines.push(file_line);
        }
    }

    print_page(&lines, options, page_counter)?;

    Ok(0)
}

fn print_page(
    lines: &Vec<FileLine>,
    options: &OutputOptions,
    page: usize,
) -> Result<usize, IOError> {
    let line_separator = options.line_separator.as_bytes();
    let page_separator = options.page_separator_char.as_bytes();

    let header: Vec<String> = header_content(options, page);
    let trailer_content: Vec<String> = trailer_content(options);
    let out: &mut Stdout = &mut stdout();

    out.lock();
    for x in header {
        out.write_all(x.as_bytes())?;
        out.write_all(line_separator)?;
    }

    let lines_written = write_columns(lines, options, out)?;

    for index in 0..trailer_content.len() {
        let x: &String = trailer_content.get(index).unwrap();
        out.write_all(x.as_bytes())?;
        if index + 1 != trailer_content.len() {
            out.write_all(line_separator)?;
        }
    }
    out.write_all(page_separator)?;
    out.flush()?;
    Ok(lines_written)
}

fn write_columns(
    lines: &Vec<FileLine>,
    options: &OutputOptions,
    out: &mut Stdout,
) -> Result<usize, IOError> {
    let line_separator = options.content_line_separator.as_bytes();

    let content_lines_per_page = if options.double_space {
        options.content_lines_per_page / 2
    } else {
        options.content_lines_per_page
    };

    let columns = options.merge_files_print.unwrap_or(get_columns(options));
    let line_width: Option<usize> = options.line_width;
    let mut lines_printed = 0;
    let feed_line_present = options.form_feed_used;
    let mut not_found_break = false;

    let across_mode = options
        .column_mode_options
        .as_ref()
        .map(|i| i.across_mode)
        .unwrap_or(false);

    let mut filled_lines: Vec<Option<&FileLine>> = Vec::new();
    if options.merge_files_print.is_some() {
        let mut offset: usize = 0;
        for col in 0..columns {
            let mut inserted = 0;
            for i in offset..lines.len() {
                let line = lines.get(i).unwrap();
                if line.file_id != col {
                    break;
                }
                filled_lines.push(Some(line));
                offset += 1;
                inserted += 1;
            }

            for _i in inserted..content_lines_per_page {
                filled_lines.push(None);
            }
        }
    }

    let table: Vec<Vec<Option<&FileLine>>> = (0..content_lines_per_page)
        .map(move |a| {
            (0..columns)
                .map(|i| {
                    if across_mode {
                        lines.get(a * columns + i)
                    } else if options.merge_files_print.is_some() {
                        *filled_lines
                            .get(content_lines_per_page * i + a)
                            .unwrap_or(&None)
                    } else {
                        lines.get(content_lines_per_page * i + a)
                    }
                })
                .collect()
        })
        .collect();

    let blank_line: FileLine = FileLine::default();
    for row in table {
        let indexes = row.len();
        for (i, cell) in row.iter().enumerate() {
            if cell.is_none() && options.merge_files_print.is_some() {
                out.write_all(
                    get_line_for_printing(&options, &blank_line, columns, i, &line_width, indexes)
                        .as_bytes(),
                )?;
            } else if cell.is_none() {
                not_found_break = true;
                break;
            } else if cell.is_some() {
                let file_line: &FileLine = cell.unwrap();

                out.write_all(
                    get_line_for_printing(&options, file_line, columns, i, &line_width, indexes)
                        .as_bytes(),
                )?;
                lines_printed += 1;
            }
        }
        if not_found_break && feed_line_present {
            break;
        } else {
            out.write_all(line_separator)?;
        }
    }

    Ok(lines_printed)
}

fn get_line_for_printing(
    options: &OutputOptions,
    file_line: &FileLine,
    columns: usize,
    index: usize,
    line_width: &Option<usize>,
    indexes: usize,
) -> String {
    // Check this condition
    let blank_line = String::new();
    let fmtd_line_number: String = get_fmtd_line_number(&options, file_line.line_number, index);

    let mut complete_line = format!(
        "{}{}",
        fmtd_line_number,
        file_line.line_content.as_ref().unwrap()
    );

    let offset_spaces: &String = &options.offset_spaces;

    let tab_count: usize = complete_line.chars().filter(|i| i == &TAB).count();

    let display_length = complete_line.len() + (tab_count * 7);

    let sep = if (index + 1) != indexes && !options.join_lines {
        &options.col_sep_for_printing
    } else {
        &blank_line
    };

    format!(
        "{}{}{}",
        offset_spaces,
        line_width
            .map(|i| {
                let min_width = (i - (columns - 1)) / columns;
                if display_length < min_width {
                    for _i in 0..(min_width - display_length) {
                        complete_line.push(' ');
                    }
                }

                complete_line.chars().take(min_width).collect()
            })
            .unwrap_or(complete_line),
        sep
    )
}

fn get_fmtd_line_number(opts: &OutputOptions, line_number: usize, index: usize) -> String {
    let should_show_line_number =
        opts.number.is_some() && (opts.merge_files_print.is_none() || index == 0);
    if should_show_line_number && line_number != 0 {
        let line_str = line_number.to_string();
        let num_opt = opts.number.as_ref().unwrap();
        let width = num_opt.width;
        let separator = &num_opt.separator;
        if line_str.len() >= width {
            format!(
                "{:>width$}{}",
                &line_str[line_str.len() - width..],
                separator,
                width = width
            )
        } else {
            format!("{:>width$}{}", line_str, separator, width = width)
        }
    } else {
        String::new()
    }
}

/// Returns a five line header content if displaying header is not disabled by
/// using `NO_HEADER_TRAILER_OPTION` option.
/// # Arguments
/// * `options` - A reference to OutputOptions
/// * `page` - A reference to page number
fn header_content(options: &OutputOptions, page: usize) -> Vec<String> {
    if options.display_header_and_trailer {
        let first_line: String = format!(
            "{} {} Page {}",
            options.last_modified_time, options.header, page
        );
        vec![
            String::new(),
            String::new(),
            first_line,
            String::new(),
            String::new(),
        ]
    } else {
        Vec::new()
    }
}

fn file_last_modified_time(path: &str) -> String {
    let file_metadata = metadata(path);
    return file_metadata
        .map(|i| {
            return i
                .modified()
                .map(|x| {
                    let datetime: DateTime<Local> = x.into();
                    datetime.format("%b %d %H:%M %Y").to_string()
                })
                .unwrap_or(String::new());
        })
        .unwrap_or(String::new());
}

/// Returns five empty lines as trailer content if displaying trailer
/// is not disabled by using `NO_HEADER_TRAILER_OPTION`option.
/// # Arguments
/// * `opts` - A reference to OutputOptions
fn trailer_content(options: &OutputOptions) -> Vec<String> {
    if options.display_header_and_trailer && !options.form_feed_used {
        vec![
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
        ]
    } else {
        Vec::new()
    }
}

/// Returns starting line number for the file to be printed.
/// If -N is specified the first line number changes otherwise
/// default is 1.
/// # Arguments
/// * `opts` - A reference to OutputOptions
fn get_start_line_number(opts: &OutputOptions) -> usize {
    opts.number.as_ref().map(|i| i.first_number).unwrap_or(1)
}

/// Returns number of lines to read from input for constructing one page of pr output.
/// If double space -d is used lines are halved.
/// If columns --columns is used the lines are multiplied by the value.
/// # Arguments
/// * `opts` - A reference to OutputOptions
fn lines_to_read_for_page(opts: &OutputOptions) -> usize {
    let content_lines_per_page = opts.content_lines_per_page;
    let columns = get_columns(opts);
    if opts.double_space {
        (content_lines_per_page / 2) * columns
    } else {
        content_lines_per_page * columns
    }
}

/// Returns number of columns to output
/// # Arguments
/// * `opts` - A reference to OutputOptions
fn get_columns(opts: &OutputOptions) -> usize {
    opts.column_mode_options
        .as_ref()
        .map(|i| i.columns)
        .unwrap_or(1)
}
