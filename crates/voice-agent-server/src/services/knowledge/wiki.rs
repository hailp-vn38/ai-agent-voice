//! Trusted Wiki publication, staged revisions and Agent-visible read policy.
use super::*;

impl KnowledgeCore {
    /// An accepted Wiki revision stays canonical while a replacement draft is
    /// edited in its own vault. Crash-left staged copies are ignored only when
    /// their content is exactly the already accepted new canonical revision.
    async fn staged_for(&self,canonical:&Record)->Result<Option<Record>,Error> {
        let Some(draft)=old_record(&self.drafts,&canonical.id).await? else {return Ok(None)};
        if draft.kind!="wiki" || draft.status!="draft" {return Err(Error::InvalidInput);}
        if draft.draft_base_revision==Some(canonical.revision) && canonical.status=="verified" {
            return Ok(Some(draft));
        }
        if canonical.status=="verified" &&
            draft.draft_base_revision.is_some_and(|base|base<canonical.revision) &&
            draft.title==canonical.title && draft.body==canonical.body {
            return Ok(None);
        }
        Err(Error::RevisionConflict)
    }
    async fn edit_record(&self,id:&str)->Result<Record,Error>{
        let canonical=self.vault.get(id).await?;
        if canonical.kind!="wiki"{return Err(Error::NotFound);}
        Ok(self.staged_for(&canonical).await?.unwrap_or(canonical))
    }
    fn range(mut rec:Record,from_line:usize,max_lines:usize)->Result<Record,Error>{
        if from_line==0 || !(1..=10000).contains(&max_lines){return Err(Error::InvalidInput);}
        if from_line!=1 || rec.body.lines().count()>max_lines {
            rec.body=rec.body.lines().skip(from_line-1).take(max_lines)
                .collect::<Vec<_>>().join("\n");
        }
        Ok(rec)
    }
    /// Admin-only list of canonical Wiki drafts and published pages. Job tools
    /// only receive verified documents through search/read_verified.
    pub async fn admin_list(&self, limit: usize) -> Result<Vec<PageOverview>, Error> {
        if !(1..=200).contains(&limit) {
            return Err(Error::InvalidInput);
        }
        let _guard = self.vault.writer.lock().await;
        let mut pages = Vec::new();
        for scanned in self.vault.scan().await? {
            let record = self.staged_for(&scanned.record).await?.unwrap_or(scanned.record);
            if record.kind != "wiki" {
                return Err(Error::InvalidInput);
            }
            pages.push(PageOverview {
                id: record.id,
                title: record.title,
                revision: record.revision,
                status: record.status,
                source_ref: record.source_ref,
                excerpt: record.body.chars().take(600).collect(),
            });
        }
        pages.truncate(limit);
        Ok(pages)
    }
    pub async fn import_source(&self, input: SourceInput) -> Result<Source, Error> {
        self.sources.import_markdown(input).await
    }
    pub async fn list_sources(&self) -> Result<Vec<Source>, Error> {
        self.sources.list().await
    }
    pub async fn read_source(&self, id: &str) -> Result<String, Error> {
        self.sources.read_text(id).await
    }
    /// Publish a new or replacement draft after independently verifying Source
    /// evidence and the expected draft revision (optimistic CAS).
    pub async fn publish(&self,id:&str,expected_revision:i64,source_ref:&str)->Result<Record,Error>{
        let _guard=self.vault.writer.lock().await;
        let canonical=self.vault.get(id).await?;
        if canonical.kind!="wiki" {return Err(Error::NotFound);}
        let staged=self.staged_for(&canonical).await?;
        let (draft,revision)=if let Some(draft)=staged.as_ref(){
            // The staging revision is a separate CAS space. The canonical Wiki
            // revision advances exactly once when the revision is accepted.
            next_revision(Some(draft),Some(expected_revision))?;
            if canonical.status!="verified" {return Err(Error::RevisionConflict);}
            (draft.clone(),canonical.revision.checked_add(1).ok_or(Error::InvalidInput)?)
        } else {
            if canonical.status!="draft" {return Err(Error::InvalidInput);}
            let revision=next_revision(Some(&canonical),Some(expected_revision))?;
            (canonical.clone(),revision)
        };
        self.sources.verify(source_ref).await?;
        let mut rec=draft;
        rec.revision=revision;
        rec.status="verified".into();
        rec.draft_base_revision=None;
        rec.source_ref=Some(source_ref.to_owned());
        self.index.mark_stale().await?;
        self.vault.commit(&rec,Some(&canonical)).await?;
        self.index.reconcile(&self.vault).await?;
        // A crash between canonical commit and cleanup is detected by staged_for.
        if staged.is_some() {
            if let Err(error)=self.drafts.remove_draft(id).await {
                tracing::warn!(wiki_id=%id,%error,"accepted Wiki revision has pending draft cleanup");
            }
        }
        Ok(rec)
    }
    /// Updating published Wiki creates a *staged* draft; the accepted revision
    /// and Agent-readable FTS projection remain unchanged until publication.
    pub async fn put_draft(&self,input:WikiInput)->Result<Record,Error>{
        check_text(&input.title,&input.body)?;
        let id=input.id.unwrap_or_else(||format!("wiki_{}",Uuid::new_v4().simple()));
        let _guard=self.vault.writer.lock().await;
        let previous=old_record(&self.vault,&id).await?;
        if previous.as_ref().is_some_and(|r|r.kind!="wiki") {return Err(Error::InvalidInput);}
        if let Some(canonical)=previous.as_ref().filter(|r|r.status=="verified"){
            let staged=self.staged_for(canonical).await?;
            let revision=if let Some(draft)=staged.as_ref(){
                next_revision(Some(draft),input.expected_revision)?
            } else {
                let next=next_revision(Some(canonical),input.expected_revision)?;
                // Remove a crash-left staged copy of an already published revision
                // before starting a new draft against this canonical base.
                if old_record(&self.drafts,&id).await?.is_some() {
                    self.drafts.remove_draft(&id).await?;
                }
                next
            };
            let rec=Record {
                id,kind:"wiki".into(),title:input.title,body:input.body,
                revision,status:"draft".into(),scope:None,confirmed:false,
                source_ref:input.source_ref,
                draft_base_revision:Some(canonical.revision)
            };
            self.drafts.commit(&rec,staged.as_ref()).await?;
            return Ok(rec);
        }
        let revision=next_revision(previous.as_ref(),input.expected_revision)?;
        let rec=Record {
            id,kind:"wiki".into(),title:input.title,body:input.body,
            revision,status:"draft".into(),scope:None,confirmed:false,
            source_ref:input.source_ref,draft_base_revision:None
        };
        self.index.mark_stale().await?;
        self.vault.commit(&rec,previous.as_ref()).await?;
        self.index.reconcile(&self.vault).await?;
        Ok(rec)
    }
    /// Line range is for browsing only; the Admin editor calls read_full.
    pub async fn read(&self,id:&str,from_line:usize,max_lines:usize)->Result<Record,Error>{
        let _guard=self.vault.writer.lock().await;
        let record=self.edit_record(id).await?;
        Self::range(record,from_line,max_lines)
    }
    pub async fn read_full(&self,id:&str)->Result<Record,Error>{
        let _guard=self.vault.writer.lock().await;
        self.edit_record(id).await
    }
    pub async fn search(&self, q: &str, limit: i64) -> Result<Vec<SearchHit>, Error> {
        let _guard = self.vault.writer.lock().await;
        let mut hits=self.index.search(q,limit).await?;
        for hit in &mut hits {
            let page=self.vault.get(&hit.id).await?;
            if page.status!="verified" || page.kind!="wiki" ||
                !self.verify_reference(page.source_ref.as_deref()).await {
                self.index.mark_stale().await?;
                return Err(Error::IndexStale);
            }
            hit.source_ref=page.source_ref;
        }
        Ok(hits)
    }
    async fn verify_reference(&self,source_ref:Option<&str>)->bool {
        match source_ref {
            Some(reference)=>self.sources.verify(reference).await.is_ok(),
            None=>false
        }
    }
    pub async fn admin_search(&self, q: &str, limit: i64) -> Result<Vec<SearchHit>, Error> {
        let _guard = self.vault.writer.lock().await;
        self.index.admin_search(q, limit).await
    }
    pub async fn read_verified(
        &self,
        id: &str,
        from_line: usize,
        max_lines: usize,
    ) -> Result<Record, Error> {
        // Never read staged edits through an Agent operation.
        let _guard=self.vault.writer.lock().await;
        let page=self.vault.get(id).await?;
        if page.kind!="wiki" || page.status!="verified"{return Err(Error::NotFound);}
        if !self.verify_reference(page.source_ref.as_deref()).await {
            self.index.mark_stale().await?;
            return Err(Error::IndexStale);
        }
        Self::range(page,from_line,max_lines)
    }
    pub async fn health(&self) -> Result<Health, Error> {
        self.index.health().await
    }
    pub async fn rebuild(&self) -> Result<usize, Error> {
        let _guard = self.vault.writer.lock().await;
        for scanned in self.vault.scan().await? {
            if scanned.record.status == "verified" &&
                !self.verify_reference(scanned.record.source_ref.as_deref()).await {
                self.index.mark_stale().await?;
                return Err(Error::IndexStale);
            }
        }
        self.index.rebuild(&self.vault).await
    }
}
