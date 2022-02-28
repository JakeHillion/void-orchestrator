fn main() {
    let mut args = std::env::args();

    let _bin = args.next();

    match args.next() {
        Some(s) => match s.as_str() {
            "main1" => main1(),
            "main2" => main2(),
            _ => unimplemented!(),
        },
        None => unimplemented!(),
    }
}

fn main1() {
    println!("hello from main1!");
}

fn main2() {
    println!("hello from main2!");
}
