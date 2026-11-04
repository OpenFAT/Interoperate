//! `pdk-scan`: the `pdk-scan` protocol (`crate::pdk_scan`), the kit's files read from stdin
//! with `--stdin`, the answer written to stdout.

/// Runs `pdk-scan` on `args`; its exit status, 2 when stdout cannot be written.
pub(super) fn pdk_scan(args: &[String]) -> u8 {
    use std::io::{Read, Write};
    let mut input = Vec::new();
    let piped = args.iter().any(|a| a == "--stdin");
    let (out, code) = if piped && std::io::stdin().read_to_end(&mut input).is_err() {
        (b"pdkscan 1\nerror \"cannot read stdin\"\nend\n".to_vec(), 2)
    } else {
        crate::pdk_scan::run(args, &input)
    };
    let mut stdout = std::io::stdout().lock();
    if stdout
        .write_all(&out)
        .and_then(|()| stdout.flush())
        .is_err()
    {
        return 2;
    }
    code
}
