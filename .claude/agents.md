# Autonomous Agent Instructions

Project: rust
Primary task: deploy

This project is configured for autonomous AI agent workflows with human-in-the-loop approval.

## Response Format

**Every response MUST end with one of these signals:**

### `runFlowTask: deploy (.)`
Use after implementing code changes. This is the PRIMARY signal - most responses should end with this.

Examples:
```
runFlowTask: deploy (.)
runFlowTask: test (.)
runFlowTask: commitWithCheck (.)
```

### `notify: <message>`
Use ONLY when you cannot proceed or need human input. This pauses the autonomous flow.

Examples:
```
notify: Need clarification on the database schema
notify: Cannot proceed - missing API key
notify: Build failed - requires manual fix
```

## Critical Rule: Implementation → deploy

**After implementing ANY code change, ALWAYS end with:**
```
runFlowTask: deploy (.)
```

This ensures the code gets built and deployed. The human will approve via the widget.

## Flow Priority

1. **Code change made** → `runFlowTask: deploy (.)`
2. **Tests needed** → `runFlowTask: test (.)`
3. **Ready to commit** → `runFlowTask: commitWithCheck (.)`
4. **Blocked/need input** → `notify: <reason>`

## Examples

### After implementing a feature
```
Done. Added the new command.

runFlowTask: deploy (.)
```

### After fixing a bug
```
Fixed the null pointer exception.

runFlowTask: deploy (.)
```

### When blocked
```
notify: Cannot implement - need database connection string
```

## Available Flow Tasks

Run `f tasks` to see all available tasks for this project.
