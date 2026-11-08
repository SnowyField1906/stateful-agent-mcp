mod east {
    struct Token;

    impl Token {
        fn parse(&self) {}
    }
}

mod west {
    struct Token;

    impl Token {
        fn parse(&self) {}
    }
}

fn ambiguous(token: east::Token) {
    token.parse();
}
