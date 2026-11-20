# Upgrade from the released client's state directory (2026-11-19)

What was checked: that a whole state directory written by a released client, whose window
was the Rune interface, is loaded by the new model as the start of the application loads it,
and closed without loss. Until now only the preferences file had a model test.

This is a model test on a copy. Nothing here was seen in a window, no packaged build was
installed over an old one, and no host was contacted.

## Method

The ignored test `a_state_directory_of_the_released_client_loads_and_closes_without_loss`
of `crates/xenadmin-ui/src/upgrade_tests.rs`:

```sh
XENADMIN_UPGRADE_STATE_DIR=/path/to/state cargo test -p xenadmin-ui --locked \
    a_state_directory_of_the_released_client -- --ignored --nocapture
```

- It copies the regular files of the directory into a temporary directory and runs the
  load in a second process whose state directory is the copy, whose home is a temporary
  directory and which has no session bus. At the end it reads the original again and fails
  if a byte differs.
- The load is the application's: `launch::prepare`, `launch::attach`,
  `Model::live(owner, Preferences::load())`, `Model::setup`; the close is `Model::close`.
- In a unit test of the model crate a keyring job never reaches a keyring, so the start-up
  restore of a saved connection is asked for and then waits: no connection is made.
- The core opens the journal only for a session with a pool. For the journal part the model
  connects to the crate's fixture server on the loopback interface, which presents the pool
  the journal's records belong to. The journal is read before and after with SQLite
  directly, on a copy of its files.
- The test prints counts and yes or no; a failed assertion names a field and never a value.

## Result

Two directories outside the repository, both written against the project's lab only.

| | written by 0.19 | written by 0.17 |
| --- | --- | --- |
| Files | 3 (preferences, profiles, lock) | 4 (the same and the journal) |
| Saved connections listed, each field as the file has it (endpoint, user, certificate pins, verified members, pool, credential mode) | yes: 1 connection, 1 pin | yes: 1 connection, 2 pins |
| Preferences read as the file has them | yes: 13 keys, and `inspector_width`, which is not kept | yes: 13 keys, and `inspector_width` |
| The connect form starts from the remembered endpoint, user and fingerprint | yes | yes |
| Connections asked to restore at start (those that opted in with a saved password) | 0 | 1; none connected |
| The close wrote `preferences.json` without `inspector_width` and changed nothing else of it | yes | yes |
| `profiles.json` byte for byte the same after start and close | yes | yes |
| Journal byte for byte the same after start and close (it is not opened without a session) | no journal | yes |
| Journal after a session of its pool opened it | no journal | format 5 (was 4); 1 VM operation before and after, listed by the model; payload byte for byte the same; 0 resource operations, 0 batch members |
| Original directory unchanged | yes | yes |

No loss was found.

## What this does not show

- A directory of a release older than 0.17, a journal with unfinished or unknown operations,
  resource operations or batches: the one journal at hand holds one finished VM operation.
- The keyring: whether a password saved by the released client is found by the new one was
  not tried, because the test must not read a keyring. The key under which a password is
  saved is the core's, which the migration did not change.
- The upgrade of an installed package over an installed package.
