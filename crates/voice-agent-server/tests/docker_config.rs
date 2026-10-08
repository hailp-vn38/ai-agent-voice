use voice_agent_server::config::AppConfig;

#[test]
fn production_docker_config_uses_container_paths_and_listener() {
    let config: AppConfig =
        toml::from_str(include_str!("../../../docker/config.prod.example.toml")).unwrap();
    config.validate().unwrap();
    assert_eq!(config.server.bind.to_string(), "0.0.0.0:8000");
    assert!(config.api.enabled);
    assert_eq!(
        config.runtime.onnx.library.to_str().unwrap(),
        "/app/runtime/onnxruntime/libonnxruntime.so"
    );
}

#[test]
fn docker_qualification_config_is_compile_time_only() {
    let parsed = toml::from_str::<AppConfig>(include_str!("../../../docker/config.test.toml"));
    #[cfg(feature = "qualification-providers")]
    {
        let config = parsed.unwrap();
        config.validate().unwrap();
        assert_eq!(config.server.bind.to_string(), "0.0.0.0:8000");
        assert!(config.api.enabled);
    }
    #[cfg(not(feature = "qualification-providers"))]
    assert!(parsed.is_err());
}
