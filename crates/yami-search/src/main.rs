use std::{env, path::Path, process::ExitCode};
use yami_search::{Index, Provider, Publication, Result};

fn run(args: &[String]) -> Result<()> {
    match args {
        [cmd, directory, flag, output] if cmd == "export" && flag == "--output" => Publication::build(&Index::open(directory)?)?.export(output),
        [cmd, directory] if cmd == "verify" => { println!("{:?}", Publication::load(directory)?.stats()); Ok(()) },
        [cmd, sub, directory] if cmd == "provider" && sub == "create" => Provider::create(directory),
        [cmd, sub, directory, index] if cmd == "provider" && sub == "add" => {
            let mut provider = Provider::open(directory)?;
            provider.add(&Index::open(index)?)?;
            provider.save(directory)
        }
        [cmd, sub, directory] if cmd == "provider" && sub == "build" => Provider::open(directory)?.build()?.export(Path::new(directory).join("published")),
        [cmd, sub, directory, flag, output] if cmd == "provider" && sub == "build" && flag == "--output" => Provider::open(directory)?.build()?.export(output),
        [cmd, root, flag, output] if cmd == "index" && flag == "--output" => Index::build(root, None)?.save(output),
        [cmd, root, flag, output, idflag, id] if cmd == "index" && flag == "--output" && idflag == "--identity" => {
            let identity = id.parse().map_err(|_| std::io::Error::other("invalid full node identity"))?;
            Index::build(root, Some(identity))?.save(output)
        }
        [cmd, directory, query] if cmd == "query" => {
            for hit in Index::open(directory)?.search(query, 100)? {
                println!("{} [{}] score={} source={}\n{}\n{}\n", hit.page.title, hit.page.key, hit.score, hit.source,
                    hit.page.address.as_deref().unwrap_or("local file (no node identity supplied)"), hit.snippet);
            }
            Ok(())
        }
        [cmd, directory] if cmd == "stats" => { println!("{:?}", Index::open(directory)?.stats()); Ok(()) }
        [cmd, directory] if cmd == "rebuild" => Index::open(Path::new(directory))?.rebuild()?.save(directory),
        _ => Err(std::io::Error::other("usage: yami-search index SITE --output INDEX [--identity FULL-ID] | query INDEX QUERY | stats INDEX | rebuild INDEX")),
    }
}
fn main() -> ExitCode {
    match run(&env::args().skip(1).collect::<Vec<_>>()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("yami-search: {error}");
            ExitCode::FAILURE
        }
    }
}
