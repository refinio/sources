# Sources CLI compatibility policy

- Absolutely no backwards compatibility is required for this experimental first version.
- Internal refactors make hard cuts and update all in-repository consumers together; compatibility shims and transitional wrappers are forbidden.
- Before introducing downstream consumers, establish their compatibility requirements. Report intentional public-interface or persisted-format breaks explicitly.
