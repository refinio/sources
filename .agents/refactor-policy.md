# Sources CLI compatibility policy

Sources is experimental, MIT-licensed software intended for public source and binary distribution through GitHub. No established downstream consumers are recorded in this repository.

- Absolutely no backwards compatibility is required for this experimental first version.
- Internal refactors make hard cuts and update all in-repository consumers together; compatibility shims and transitional wrappers are forbidden.
- Before introducing downstream consumers, establish their compatibility requirements. Report intentional public-interface or persisted-format breaks explicitly.
