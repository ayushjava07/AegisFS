use std::sync::Arc;

use dashmap::DashMap;

use crate::core::error::{AegisError, AegisResult};
use crate::core::traits::*;

#[derive(Debug, Clone)]
pub struct PluginManifest {
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    pub api_version: u32,
}

impl PluginManifest {
    pub fn new(name: &str, version: &str) -> Self {
        Self {
            name: name.to_string(),
            version: version.to_string(),
            description: String::new(),
            author: String::new(),
            api_version: 1,
        }
    }

    pub fn with_description(mut self, description: &str) -> Self {
        self.description = description.to_string();
        self
    }

    pub fn with_author(mut self, author: &str) -> Self {
        self.author = author.to_string();
        self
    }

    pub fn with_api_version(mut self, version: u32) -> Self {
        self.api_version = version;
        self
    }
}

pub struct PluginRegistryImpl {
    plugins: DashMap<String, Arc<dyn Plugin>>,
}

impl PluginRegistryImpl {
    pub fn new() -> Self {
        Self {
            plugins: DashMap::new(),
        }
    }
}

impl Default for PluginRegistryImpl {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginRegistry for PluginRegistryImpl {
    fn register(&self, plugin: Arc<dyn Plugin>) -> AegisResult<()> {
        let name = plugin.name().to_string();
        if self.plugins.contains_key(&name) {
            return Err(AegisError::PluginError(format!(
                "plugin '{}' is already registered",
                name
            )));
        }
        plugin.initialize()?;
        self.plugins.insert(name.clone(), plugin);
        Ok(())
    }

    fn unregister(&self, name: &str) -> AegisResult<()> {
        let (_key, plugin) = self
            .plugins
            .remove(name)
            .ok_or_else(|| AegisError::PluginError(format!("plugin '{}' not found", name)))?;
        plugin.shutdown()?;
        Ok(())
    }

    fn get(&self, name: &str) -> AegisResult<Arc<dyn Plugin>> {
        self.plugins
            .get(name)
            .map(|e| e.value().clone())
            .ok_or_else(|| AegisError::PluginError(format!("plugin '{}' not found", name)))
    }

    fn list(&self) -> AegisResult<Vec<String>> {
        let mut names: Vec<String> = self.plugins.iter().map(|e| e.key().clone()).collect();
        names.sort();
        Ok(names)
    }
}

pub struct NoopPlugin;

impl NoopPlugin {
    pub fn new() -> Self {
        Self
    }
}

impl Default for NoopPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl Plugin for NoopPlugin {
    fn name(&self) -> &str {
        "noop"
    }

    fn version(&self) -> &str {
        "0.1.0"
    }

    fn initialize(&self) -> AegisResult<()> {
        Ok(())
    }

    fn shutdown(&self) -> AegisResult<()> {
        Ok(())
    }
}

pub struct PluginBuilder {
    manifest: PluginManifest,
    initialize_fn: Option<Box<dyn Fn() -> AegisResult<()> + Send + Sync>>,
    shutdown_fn: Option<Box<dyn Fn() -> AegisResult<()> + Send + Sync>>,
}

impl PluginBuilder {
    pub fn new(manifest: PluginManifest) -> Self {
        Self {
            manifest,
            initialize_fn: None,
            shutdown_fn: None,
        }
    }

    pub fn with_initialize<F>(mut self, f: F) -> Self
    where
        F: Fn() -> AegisResult<()> + Send + Sync + 'static,
    {
        self.initialize_fn = Some(Box::new(f));
        self
    }

    pub fn with_shutdown<F>(mut self, f: F) -> Self
    where
        F: Fn() -> AegisResult<()> + Send + Sync + 'static,
    {
        self.shutdown_fn = Some(Box::new(f));
        self
    }

    pub fn build(self) -> BuiltPlugin {
        BuiltPlugin {
            manifest: self.manifest,
            initialize_fn: self.initialize_fn,
            shutdown_fn: self.shutdown_fn,
        }
    }
}

pub struct BuiltPlugin {
    manifest: PluginManifest,
    initialize_fn: Option<Box<dyn Fn() -> AegisResult<()> + Send + Sync>>,
    shutdown_fn: Option<Box<dyn Fn() -> AegisResult<()> + Send + Sync>>,
}

impl Plugin for BuiltPlugin {
    fn name(&self) -> &str {
        &self.manifest.name
    }

    fn version(&self) -> &str {
        &self.manifest.version
    }

    fn initialize(&self) -> AegisResult<()> {
        if let Some(ref f) = self.initialize_fn {
            f()
        } else {
            Ok(())
        }
    }

    fn shutdown(&self) -> AegisResult<()> {
        if let Some(ref f) = self.shutdown_fn {
            f()
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestPlugin {
        name: String,
    }

    impl Plugin for TestPlugin {
        fn name(&self) -> &str {
            &self.name
        }
        fn version(&self) -> &str {
            "1.0.0"
        }
        fn initialize(&self) -> AegisResult<()> {
            Ok(())
        }
        fn shutdown(&self) -> AegisResult<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_register_and_list() {
        let registry = PluginRegistryImpl::new();
        let plugin = Arc::new(TestPlugin {
            name: "test-plugin".into(),
        });
        registry.register(plugin).unwrap();

        let names = registry.list().unwrap();
        assert_eq!(names, vec!["test-plugin"]);
    }

    #[tokio::test]
    async fn test_get_plugin() {
        let registry = PluginRegistryImpl::new();
        let plugin = Arc::new(TestPlugin {
            name: "my-plugin".into(),
        });
        registry.register(plugin).unwrap();

        let retrieved = registry.get("my-plugin").unwrap();
        assert_eq!(retrieved.name(), "my-plugin");
        assert_eq!(retrieved.version(), "1.0.0");
    }

    #[tokio::test]
    async fn test_unregister_plugin() {
        let registry = PluginRegistryImpl::new();
        let plugin = Arc::new(TestPlugin {
            name: "remove-me".into(),
        });
        registry.register(plugin).unwrap();
        assert!(registry.list().unwrap().contains(&"remove-me".to_string()));

        registry.unregister("remove-me").unwrap();
        assert!(!registry.list().unwrap().contains(&"remove-me".to_string()));
    }

    #[tokio::test]
    async fn test_register_duplicate_fails() {
        let registry = PluginRegistryImpl::new();
        let p1 = Arc::new(TestPlugin { name: "dup".into() });
        let p2 = Arc::new(TestPlugin { name: "dup".into() });
        registry.register(p1).unwrap();
        let result = registry.register(p2);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_get_nonexistent_fails() {
        let registry = PluginRegistryImpl::new();
        let result = registry.get("nonexistent");
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_noop_plugin() {
        let noop = NoopPlugin::new();
        assert_eq!(noop.name(), "noop");
        assert_eq!(noop.version(), "0.1.0");
        assert!(noop.initialize().is_ok());
        assert!(noop.shutdown().is_ok());
    }

    #[tokio::test]
    async fn test_plugin_builder() {
        let manifest = PluginManifest::new("built", "2.0.0")
            .with_description("A built plugin")
            .with_author("test");
        let init_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let shutdown_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let init = init_flag.clone();
        let shutdown = shutdown_flag.clone();

        let built = PluginBuilder::new(manifest)
            .with_initialize(move || {
                init.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            })
            .with_shutdown(move || {
                shutdown.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            })
            .build();

        assert_eq!(built.name(), "built");
        assert_eq!(built.version(), "2.0.0");
        built.initialize().unwrap();
        assert!(init_flag.load(std::sync::atomic::Ordering::SeqCst));
        built.shutdown().unwrap();
        assert!(shutdown_flag.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[tokio::test]
    async fn test_empty_list() {
        let registry = PluginRegistryImpl::new();
        let names = registry.list().unwrap();
        assert!(names.is_empty());
    }
}
