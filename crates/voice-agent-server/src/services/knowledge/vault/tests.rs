    use super::*;
    #[tokio::test]
    async fn orphan_snapshot_temp_does_not_block_next_revision() {
        let root=std::env::temp_dir().join(format!("wiki_revision_{}",Uuid::new_v4()));
        let vault=MarkdownVault::new(root.clone(),"wiki").await.unwrap();
        let first=Record {
            id:"wiki_revision".into(),kind:"wiki".into(),title:"T".into(),
            body:"previous bytes".into(),revision:1,status:"draft".into(),
            scope:None,confirmed:false,source_ref:None
        };
        vault.commit(&first,None).await.unwrap();
        let history=root.join("revisions").join(&first.id);
        tokio::fs::create_dir_all(&history).await.unwrap();
        tokio::fs::write(history.join("1.crash.tmp"),b"partially written").await.unwrap();
        let second=Record{revision:2,body:"new bytes".into(),..first.clone()};
        vault.commit(&second,Some(&first)).await.unwrap();
        let archived=tokio::fs::read_to_string(history.join("1.md")).await.unwrap();
        assert_eq!(archived,encode(&first).unwrap());
        drop(vault);
        let _=tokio::fs::remove_dir_all(root).await;
    }

    #[test]
    fn round_trip_and_no_path_escape() {
        let p = Record {
            id: "wiki_1".into(),
            kind: "wiki".into(),
            title: "Vietnamese: Trí nhớ".into(),
            body: "# Welcome\nText".into(),
            revision: 1,
            status: "draft".into(),
            scope: None,
            confirmed: false,
            source_ref: Some("src_1:1".into()),
        };
        let encoded = encode(&p).unwrap();
        let decoded = parse(&encoded).unwrap();
        assert_eq!(p.id, decoded.id);
        assert_eq!(p.title, decoded.title);
        assert_eq!(p.source_ref, decoded.source_ref);
        assert!(!valid_id("../secret"));
        assert!(parse("---\nid: x\n---\n").is_err());
    }

