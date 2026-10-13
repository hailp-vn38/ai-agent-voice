ALTER TABLE job_steps ADD COLUMN instruction TEXT NOT NULL DEFAULT '' CHECK(length(CAST(instruction AS BLOB))<=4000);
CREATE TRIGGER job_steps_contract_immutable BEFORE UPDATE ON job_steps
WHEN NEW.job_id IS NOT OLD.job_id OR NEW.step_key IS NOT OLD.step_key
 OR NEW.plan_revision IS NOT OLD.plan_revision OR NEW.plugin_id IS NOT OLD.plugin_id
 OR NEW.plugin_version IS NOT OLD.plugin_version OR NEW.package_digest IS NOT OLD.package_digest
 OR NEW.image_digest IS NOT OLD.image_digest OR NEW.operation IS NOT OLD.operation
 OR NEW.arguments_json IS NOT OLD.arguments_json OR NEW.input_refs_json IS NOT OLD.input_refs_json
 OR NEW.output_schema_json IS NOT OLD.output_schema_json OR NEW.dependencies_json IS NOT OLD.dependencies_json
 OR NEW.required IS NOT OLD.required OR NEW.instruction IS NOT OLD.instruction
BEGIN SELECT RAISE(ABORT,'job_step_contract_immutable'); END;
