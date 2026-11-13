-- Add api_key_id and separate idempotency from template data
ALTER TABLE email_jobs
ADD COLUMN api_key_id UUID REFERENCES api_keys(id),
ADD COLUMN idempotency_key VARCHAR(255),
ADD COLUMN template_data JSONB;

-- Drop old constraint and add new one scoped to api_key_id
ALTER TABLE email_jobs
DROP CONSTRAINT unique_reference_template;

-- New idempotency constraint scoped to API key
CREATE UNIQUE INDEX unique_api_key_idempotency 
ON email_jobs(api_key_id, idempotency_key) 
WHERE idempotency_key IS NOT NULL;

-- Index for queries scoped by API key
CREATE INDEX idx_email_jobs_api_key_id ON email_jobs(api_key_id);
