//! The WadSpaces account and the projects' secret files (`/v1/account`,
//! `/v1/secret-files`: wad-proto's `account`).

use wad_proto::{AccountLink, SecretFile, SecretFileAnswer, SecretFileKeep, SecretFiles};

use crate::{Failure, Wadd};

impl Wadd {
    /// The machine's link to its account (`GET /v1/account`).
    pub async fn account(&self) -> Result<AccountLink, Failure> {
        self.get("/v1/account").await
    }

    /// Links it with the GitHub sign-in (`POST /v1/account/link`).
    pub async fn account_link(&self) -> Result<AccountLink, Failure> {
        self.post("/v1/account/link", &()).await
    }

    /// Unlinks it (`DELETE /v1/account`).
    pub async fn account_unlink(&self) -> Result<AccountLink, Failure> {
        self.delete("/v1/account").await
    }

    /// What's kept, and what's asking (`GET /v1/secret-files`).
    pub async fn secret_files(&self) -> Result<SecretFiles, Failure> {
        self.get("/v1/secret-files").await
    }

    /// Keeps a file (`POST /v1/secret-files`): `value` is its bytes in base64.
    pub async fn secret_file_keep(&self, keep: &SecretFileKeep) -> Result<SecretFile, Failure> {
        self.post("/v1/secret-files", keep).await
    }

    /// No longer kept (`DELETE /v1/secret-files/{name}`).
    pub async fn secret_file_remove(&self, name: &str) -> Result<(), Failure> {
        self.act(reqwest::Method::DELETE, &format!("/v1/secret-files/{name}"), None::<&()>).await
    }

    /// Answers an ask (`POST /v1/secret-files/asks/{id}`): keep it, or leave it be.
    pub async fn secret_file_answer(&self, id: &str, keep: bool) -> Result<SecretFiles, Failure> {
        self.post(&format!("/v1/secret-files/asks/{id}"), &SecretFileAnswer { keep }).await
    }
}
