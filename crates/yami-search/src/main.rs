use std::{env, path::Path, process::ExitCode};
use yami_search::{
    AuthenticatedFetcher, Index, Provider, Publication, Result, SearchStore, WebsiteAddress,
};

fn run(args: &[String]) -> Result<()> {
    match args {
        [cmd, directory, name, address] if cmd == "subscribe" => SearchStore::open(directory)?.add(name, WebsiteAddress::parse(address).map_err(|e| std::io::Error::other(e.to_string()))?),
        [cmd, directory, name] if cmd == "unsubscribe" => SearchStore::open(directory)?.remove(name),
        [cmd, directory] if cmd == "subscriptions" => {
            for provider in SearchStore::open(directory)?.providers() { println!("{} {} {:?}", provider.name, provider.address, provider.status()); }
            Ok(())
        }
        [cmd, directory, query] if cmd == "search" => {
            for result in SearchStore::open(directory)?.search(query, None, 100)? {
                println!("{} score={} {}\n{}\nProviders: {:?} {:?}\n", result.hit.page.title, result.hit.score, result.hit.page.address.as_deref().unwrap_or(""), result.hit.snippet, result.providers, result.status);
            }
            Ok(())
        }
        [cmd, directory, name, peers] if cmd == "refresh" => refresh(directory, name, peers, false),
        [cmd, directory, name, peers, flag] if cmd == "refresh" && flag == "--relays" => refresh(directory, name, peers, true),
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
        _ => Err(std::io::Error::other("usage: yami-search index SITE --output INDEX [--identity FULL-ID] | query INDEX QUERY | stats INDEX | rebuild INDEX | export INDEX --output NEW-DIR | verify PUBLICATION | provider create DIR | provider add DIR INDEX | provider build DIR [--output NEW-DIR] | subscribe STATE NAME yami://FULL-ID/search-manifest | unsubscribe STATE NAME | subscriptions STATE | refresh STATE NAME PEER-DIR [--relays] | search STATE QUERY")),
    }
}
fn refresh(directory: &str, name: &str, peers: &str, relays: bool) -> Result<()> {
    let mut store = SearchStore::open(directory)?;
    let bootstrap =
        yami_discover::load_bootstrap_peers(Path::new(peers), yami_dht::Network::Public)
            .map_err(|e| std::io::Error::other(e.to_string()))?;
    let fetcher = AuthenticatedFetcher::open_with_relays(
        directory,
        yami_dht::Network::Public,
        bootstrap,
        relays,
    )
    .map_err(|e| std::io::Error::other(e.to_string()))?;
    store.refresh(name, &fetcher, &std::sync::atomic::AtomicBool::new(false))
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
