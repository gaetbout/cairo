use std::{fs, path::PathBuf};

use genco::prelude::*;
use xshell::{cmd, Shell};

use super::spec::{get_spec, Member, Node, NodeKind};

pub fn project_root() -> PathBuf {
    let dir = env!("CARGO_MANIFEST_DIR");
    let res = PathBuf::from(dir).parent().unwrap().parent().unwrap().to_owned();
    println!("res: {:?}", res);
    assert!(res.join("Cargo.toml").exists());
    res
}

#[test]
fn sourcegen_ast() {
    let results = [
        ensure_file_content("crates/syntax/src/node/ast.rs", generate_ast_code()),
        ensure_file_content("crates/syntax/src/node/kind.rs", generate_kinds_code()),
    ];
    if results.into_iter().any(|x| x) {
        panic!("Some file was not up to date and has been updated, simply re-run the tests.");
    }
}

fn ensure_file_content(suffix: &'static str, tokens: rust::Tokens) -> bool {
    let filename = project_root().join(suffix);
    let expected_content = reformat(tokens.to_string().unwrap());
    if let Ok(old_contents) = fs::read_to_string(&filename) {
        if old_contents == expected_content {
            return false;
        }
    }

    fs::write(&filename, expected_content).unwrap();
    true
}

fn generate_kinds_code() -> rust::Tokens {
    let spec = get_spec();
    let mut tokens = quote! {
        $("// Autogenerated file.\n")
    };
    let mut kinds = rust::Tokens::new();

    // SyntaxKind.
    for Node { name, kind } in spec.into_iter() {
        match kind {
            NodeKind::Enum { variants: _, missing_variant: _ } => {}
            _ => {
                kinds.extend(quote! {
                    $name,
                });
            }
        }
    }
    tokens.extend(quote! {
        #[derive(Copy, Clone, Debug, Hash, PartialEq, Eq)]
        pub enum SyntaxKind{
            $kinds
        }
    });
    tokens
}

fn generate_ast_code() -> rust::Tokens {
    let spec = get_spec();
    let mut tokens = quote! {
        $("// Autogenerated file.\n")
        use std::ops::Deref;

        use super::kind::SyntaxKind;
        use super::element_list::ElementList;
        use super::{TypedSyntaxNode, GreenId, GreenInterner, GreenInternalNode, GreenNode, SyntaxNode, SyntaxToken};
    };
    for Node { name, kind } in spec.into_iter() {
        tokens.extend(match kind {
            NodeKind::Enum { variants, missing_variant } => {
                gen_enum_code(name, variants, missing_variant)
            }
            NodeKind::Struct { members } => gen_struct_code(name, members),
            NodeKind::List { element_type } => gen_list_code(name, element_type, 1),
            NodeKind::SeparatedList { element_type } => gen_list_code(name, element_type, 2),
        })
    }
    tokens
}

fn gen_list_code(name: String, element_type: String, step: usize) -> Tokens<Rust> {
    quote! {
        pub struct $(&name)(ElementList<$(&element_type),$step>);
        impl Deref for $(&name){
            type Target = ElementList<$(&element_type),$step>;

            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }
        impl $(&name){
            pub fn new_green(db: &dyn GreenInterner, children: Vec<GreenId>) -> GreenId{
                let width = children.iter().map(|id|
                    db.lookup_intern_green(*id).width()).sum();
                db.intern_green(GreenNode::Internal(GreenInternalNode{
                    kind: SyntaxKind::$(&name),
                    children,
                    width
                }))
            }
        }
        impl TypedSyntaxNode for $(&name) {
            fn missing(db: &dyn GreenInterner) -> GreenId {
                db.intern_green(
                    GreenNode::Internal(GreenInternalNode {
                        kind: SyntaxKind::$(&name), children: vec![], width: 0
                    })
                )
            }
            fn from_syntax_node(db: &dyn GreenInterner, node: SyntaxNode) -> Self {
                Self(ElementList::new(node))
            }
        }
    }
}

pub fn reformat(text: String) -> String {
    let sh = Shell::new().unwrap();
    sh.set_var("RUSTUP_TOOLCHAIN", "stable");
    let rustfmt_toml = project_root().join("rustfmt.toml");
    let mut stdout = cmd!(sh, "rustfmt --config-path {rustfmt_toml}").stdin(text).read().unwrap();
    if !stdout.ends_with('\n') {
        stdout.push('\n');
    }
    stdout
}

fn gen_enum_code(name: String, variants: Vec<String>, missing_variant: String) -> rust::Tokens {
    quote! {
        pub enum $(&name){
            $(for v in &variants => $v($v),)
        }
        impl TypedSyntaxNode for $(&name){
            fn missing(db: &dyn GreenInterner) -> GreenId{
                db.intern_green(GreenNode::Internal(GreenInternalNode{
                    kind: SyntaxKind::$(&missing_variant),
                    children: vec![],
                    width: 0
                }))
            }
            fn from_syntax_node(db: &dyn GreenInterner, node: SyntaxNode) -> Self{
                let green = db.lookup_intern_green(node.0.green);
                if let GreenNode::Internal(internal) = green{
                    match internal.kind{
                        $(for v in &variants => SyntaxKind::$v => return $(&name)::$v($v::from_syntax_node(db, node)),)
                        _ => panic!("Unexpected syntax kind {:?} when constructing $(&name).", internal.kind)
                    }
                }
                panic!("Expected an internal node.");
            }
        }
    }
}

fn gen_struct_code(name: String, members: Vec<Member>) -> rust::Tokens {
    let mut body = rust::Tokens::new();
    let mut arg_decls = quote! {};
    let mut args = quote! {};
    let mut arg_missings = quote! {};
    for (i, Member { name, kind }) in members.iter().enumerate() {
        arg_decls.extend(quote! {$name: GreenId,});
        args.extend(quote! {$name,});
        // TODO(spapini): Validate that children SyntaxKinds are as expected.
        match kind {
            super::spec::MemberKind::Token => {
                body.extend(quote! {
                    pub fn $name(&self, db: &dyn GreenInterner) -> SyntaxToken{
                        let child = self.children[$i].clone();
                        SyntaxToken::from_syntax_node(db, child)
                    }
                });
                arg_missings.extend(quote! {SyntaxToken::missing(db),});
            }
            super::spec::MemberKind::Node(node) => {
                body.extend(quote! {
                    pub fn $name(&self, db: &dyn GreenInterner) -> $node{
                        let child = self.children[$i].clone();
                        $node::from_syntax_node(db, child)
                    }
                });
                arg_missings.extend(quote! {$node::missing(db),});
            }
        };
    }
    quote! {
        pub struct $(&name){
            node: SyntaxNode,
            children: Vec<SyntaxNode>,
        }
        impl $(&name) {
            pub fn new_green(db: &dyn GreenInterner, $arg_decls) -> GreenId{
                let children: Vec<GreenId> = vec![$args];
                let width = children.iter().map(|id|
                    db.lookup_intern_green(*id).width()).sum();
                db.intern_green(GreenNode::Internal(GreenInternalNode{
                    kind: SyntaxKind::$(&name),
                    children,
                    width
                }))
            }
            $body
        }
        impl TypedSyntaxNode for $(&name){
            fn missing(db: &dyn GreenInterner) -> GreenId{
                let children: Vec<GreenId> = vec![$arg_missings];
                db.intern_green(GreenNode::Internal(GreenInternalNode{
                    kind: SyntaxKind::$(&name),
                    children,
                    width: 0
                }))
            }
            fn from_syntax_node(db: &dyn GreenInterner, node: SyntaxNode) -> Self{
                let green = db.lookup_intern_green(node.0.green);
                match green {
                    // Note: A missing syntax element should result in an internal green node
                    // of width 0, with as much structure as possible.
                    GreenNode::Internal(internal) => {
                        if internal.kind != SyntaxKind::$(&name){
                            panic!("Unexpected SyntaxKind {:?}. Expected {:?}.", internal.kind, SyntaxKind::$(&name));
                        }
                        let children = node.children(db);
                        Self{ node, children }
                    },
                    GreenNode::Token(token) => {
                        panic!("Unexpected Token {:?}. Expected {:?}.", token, SyntaxKind::$(&name));
                    }
                }
            }
        }
    }
}
