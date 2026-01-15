# Prompt Generation System - Complete Audit

## Overview
This document provides a comprehensive audit of the prompt-to-workflow generation system in LAO.

## Components

### 1. System Prompt (`core/prompt_dispatcher/prompt/system_prompt.txt`)

**Status**: ✅ **FIXED**

**Issues Found & Fixed**:
- ❌ **Was using**: `Whisper`, `Summarizer`, `Tagger` (incorrect plugin names)
- ✅ **Now uses**: `WhisperPlugin`, `SummarizerPlugin` (correct plugin names)
- ❌ **Was using**: `input_from: Whisper` (plugin name reference)
- ✅ **Now uses**: `input_from: step1` (step reference)
- ❌ **Was mentioning**: Non-existent plugins (`TaggerPlugin`, `CodeRefactorPlugin`)
- ✅ **Now lists**: Only available plugins (`EchoPlugin`, `WhisperPlugin`, `SummarizerPlugin`, `PromptDispatcherPlugin`, `GGUFPlugin`, `LMStudioPlugin`, `OllamaPlugin`)

**Current State**:
- Correctly instructs LLM to use step references (`step1`, `step2`, etc.)
- Lists only available plugins
- Provides clear example with correct syntax

### 2. Prompt Library JSON (`core/prompt_dispatcher/prompt/prompt_library.json`)

**Status**: ⚠️ **PARTIALLY FIXED** (some examples still have issues)

**Issues Found**:

#### Fixed Examples:
- ✅ "Summarize this audio and create a todo list" - Now uses `step1` references
- ✅ "Process video file..." - Now uses step references
- ✅ "Voice command processing..." - Now uses step references

#### Remaining Issues:
- ❌ **Example 2**: "Audit this config file for security risks"
  - Uses non-existent plugins: `ConfigParser`, `SecurityAuditor`, `Reporter`
  - Uses `input_from: ConfigParser` instead of `step1`
  
- ❌ **Example 3**: "Refactor this Rust file and add comments"
  - Uses non-existent plugins: `RustRefactor`, `CommentGenerator`
  - Uses `input_from: RustRefactor` instead of `step1`

- ❌ **Example 6**: "Analyze image and generate description with fallback"
  - Uses non-existent plugins: `ImageAnalyzer`, `ImageDescriptor`, `FallbackProcessor`
  - Uses `input_from: ImageAnalyzer` instead of step references
  - Uses `field: "ImageAnalyzer"` in conditions (should be step references)

- ❌ **Example 7**: "Smart document processing with conditional branching"
  - Uses non-existent plugins: `FileTypeDetector`, `PDFProcessor`, `ImageProcessor`, `TextProcessor`
  - Uses `input_from: FileTypeDetector` instead of step references
  - Uses `depends_on: ["PDFProcessor", "ImageProcessor", "TextProcessor"]` (plugin names instead of step references)
  - Uses `field: "FileTypeDetector"` in conditions

- ❌ **Example 8**: "Multi-modal content analysis with error handling"
  - Uses non-existent plugins: `ContentDetector`, `AudioAnalyzer`, `ImageAnalyzer`, `ErrorHandler`, `ResultAggregator`
  - Uses plugin names in `input_from` and `depends_on`
  - Uses plugin names in condition fields

- ❌ **Example 9**: "Automated file backup with compression"
  - Uses non-existent plugins: `FileSizeChecker`, `CompressorPlugin`, `DirectCopy`, `BackupUploader`
  - Uses plugin names in references

**Recommendation**: Update all examples to:
1. Use only available plugins (`EchoPlugin`, `WhisperPlugin`, `SummarizerPlugin`)
2. Use step references (`step1`, `step2`) instead of plugin names
3. Remove examples that require non-existent plugins

### 3. Prompt Library Markdown (`core/prompt_dispatcher/prompt/prompt_library.md`)

**Status**: ❌ **OUTDATED**

**Issues**:
- Still shows old examples with incorrect plugin names (`MarkdownSummarizer`, `Tagger`, `Whisper`, `Summarizer`)
- Uses plugin names in `input_from` instead of step references
- Not synchronized with JSON version

**Recommendation**: Update to match JSON or remove (if JSON is source of truth)

### 4. PromptDispatcherPlugin Implementation (`plugins/PromptDispatcherPlugin/src/lib.rs`)

**Status**: ✅ **WORKING** (with limitations)

**How It Works**:
1. **Input Validation**: Checks for "nonsense" or very short inputs
2. **Prompt Library Matching**: Tries to match input against `prompt_library.json` first
3. **LLM Fallback**: If no match, uses Ollama with `system_prompt.txt`
4. **Output Cleaning**: Removes markdown code fences from LLM output

**Issues**:
- ⚠️ **Hardcoded Model**: Uses `llama2` - should be configurable
- ⚠️ **No Error Recovery**: If Ollama fails, returns generic error
- ⚠️ **Simple Matching**: `find_matching_workflow` uses substring matching (case-insensitive) - could match incorrectly
- ⚠️ **Output Cleaning**: Only removes lines starting with ```` - might miss other markdown formatting

**Code Flow**:
```
User Prompt → PromptDispatcherPlugin.run()
  → Check input validity
  → Try prompt library match (substring search)
  → If match found: return workflow from library
  → If no match: call Ollama with system prompt
  → Clean output (remove markdown fences)
  → Return YAML workflow
```

### 5. CLI Integration (`cli/src/main.rs`)

**Status**: ✅ **WORKING**

**How It Works**:
1. Loads `PromptDispatcherPlugin` from registry
2. Calls plugin with user prompt
3. Strips code fences from output
4. Validates YAML can be parsed as `Workflow`
5. Saves to `workflows/generated_from_prompt.yaml` (or specified path)

**Functions**:
- `strip_code_fences()`: Removes markdown code fences (```yaml, ```, etc.)
- `normalize_yaml()`: Parses YAML to Value for comparison (used in validation)

**Issues**:
- ⚠️ **Plugin Name**: Looks for `"PromptDispatcher"` but plugin is named `"PromptDispatcherPlugin"` - **POTENTIAL BUG**
- ✅ **Error Handling**: Validates YAML before saving
- ✅ **Path Resolution**: Creates directories if needed

### 6. Workflow Loading (`ui/lao-ui/src/backend.rs`)

**Status**: ✅ **FIXED**

**Recent Fixes**:
- ✅ Node IDs now use `PluginName_Index` format instead of plugin name
- ✅ Each step gets unique ID even if using same plugin
- ✅ Step references (`step1`, `step2`) correctly mapped to node IDs

### 7. Workflow Files

**Status**: ✅ **ALL CORRECT**

**Verified**:
- All workflows use correct plugin names (`EchoPlugin`, `WhisperPlugin`, `SummarizerPlugin`)
- All workflows use step references (`step1`, `step2`) in `input_from` and `depends_on`
- No workflows use plugin names in references
- All workflows are logically structured

## Critical Issues Found

### ✅ **FIXED**: Plugin Name Mismatch

**Location**: `cli/main.rs:351` and `cli/main.rs:427`

**Problem Found**: 
- CLI was looking for `"PromptDispatcher"`
- Plugin is actually named `"PromptDispatcherPlugin"`

**Fix Applied**: 
- Changed both occurrences to `registry.plugins.get("PromptDispatcherPlugin")`
- ✅ **FIXED** - Prompt generation should now work correctly

### ✅ **FIXED**: Prompt Library Examples Use Non-Existent Plugins

**Problem Found**: 
- Examples in `prompt_library.json` referenced plugins that don't exist (`ConfigParser`, `RustRefactor`, `ImageAnalyzer`, etc.)

**Fix Applied**: 
- ✅ Updated all examples to use only available plugins (`EchoPlugin`, `WhisperPlugin`, `SummarizerPlugin`)
- ✅ All examples now use correct step references (`step1`, `step2`) instead of plugin names
- ✅ Condition fields now use step references instead of plugin names

### ✅ **FIXED**: Prompt Library Examples Use Plugin Names in References

**Problem Found**: 
- Examples showed `input_from: WhisperPlugin` instead of `input_from: step1`
- Examples showed `depends_on: ["PluginName"]` instead of `depends_on: ["step1"]`

**Fix Applied**: 
- ✅ All examples now use step references (`step1`, `step2`, etc.)
- ✅ All `input_from` fields use step references
- ✅ All `depends_on` arrays use step references
- ✅ All condition `field` values use step references

## Recommendations

### Immediate Fixes (Critical)

1. **Fix plugin name lookup** in CLI:
   ```rust
   // Change from:
   registry.plugins.get("PromptDispatcher")
   // To:
   registry.plugins.get("PromptDispatcherPlugin")
   ```

2. **Update prompt library examples**:
   - Replace all non-existent plugins with `EchoPlugin`
   - Change all `input_from: PluginName` to `input_from: step1`
   - Change all `depends_on: ["PluginName"]` to `depends_on: ["step1"]`
   - Update condition fields to use step references

### Short-Term Improvements

1. **Make Ollama model configurable**:
   - Add config file or environment variable
   - Allow user to specify model name

2. **Improve prompt library matching**:
   - Use better matching algorithm (fuzzy matching, semantic similarity)
   - Add confidence scores

3. **Better error messages**:
   - Provide more specific errors when Ollama fails
   - Show which part of generation failed

4. **Synchronize prompt_library.md with JSON**:
   - Either auto-generate MD from JSON
   - Or remove MD if JSON is source of truth

### Long-Term Enhancements

1. **Add workflow validation**:
   - Validate generated workflows before saving
   - Check plugin availability
   - Verify step references are valid

2. **Add workflow templates**:
   - Pre-defined workflow patterns
   - User can select template and customize

3. **Improve LLM integration**:
   - Support multiple LLM backends (not just Ollama)
   - Add streaming responses
   - Better prompt engineering

## Testing

### Current Test Coverage

✅ **Tests Exist**:
- `test_prompt_to_workflow_success` - Tests successful generation
- `test_prompt_to_workflow_failure` - Tests error handling
- `test_prompt_library_pairs` - Validates prompt library examples

⚠️ **Test Issues**:
- Tests might fail due to prompt library examples using wrong syntax
- Tests don't verify step references are correct
- Tests don't check for non-existent plugins

### Recommended Additional Tests

1. **Test plugin name lookup** (to catch the critical bug)
2. **Test step reference generation** (verify `step1`, `step2` format)
3. **Test with all available plugins** (ensure all plugins can be referenced)
4. **Test error cases** (Ollama not available, invalid YAML, etc.)

## Summary

### ✅ What's Working
- System prompt correctly instructs LLM
- Workflow loading correctly handles step references
- UI displays workflows correctly
- Basic prompt generation flow works
- ✅ **FIXED**: Plugin name lookup in CLI
- ✅ **FIXED**: All prompt library examples use correct plugins and step references
- ✅ **FIXED**: Prompt library MD synchronized with JSON

### ✅ Fixed Issues
- ✅ **Plugin name mismatch** in CLI - Fixed to use `"PromptDispatcherPlugin"`
- ✅ **Prompt library examples** - All updated to use available plugins and step references
- ✅ **Prompt library MD** - Updated to match JSON examples

### ⚠️ Remaining Medium Issues
- Ollama model is hardcoded (`llama2`) - should be configurable
- Simple substring matching for prompt library - could be improved with fuzzy/semantic matching
- No validation of generated workflows before saving (relies on YAML parsing)

### 📋 Action Items
1. ✅ **COMPLETED**: Fix plugin name lookup in CLI
2. ✅ **COMPLETED**: Update all prompt library examples
3. ✅ **COMPLETED**: Synchronize prompt_library.md
4. **LOW**: Make Ollama model configurable
5. **LOW**: Improve prompt library matching (fuzzy/semantic matching)
6. **LOW**: Add workflow validation before saving (check plugins exist, step references valid)

## Test Results

After fixes:
- ✅ CLI compiles successfully
- ✅ All prompt library examples use correct syntax
- ✅ All workflows use step references correctly
- ✅ System prompt provides clear guidance

## Conclusion

The prompt generation system has been **comprehensively audited and fixed**. All critical bugs have been resolved:
- Plugin name lookup fixed
- Prompt library examples corrected
- Documentation synchronized

The system should now generate workflows correctly with proper step references and valid plugin names.
