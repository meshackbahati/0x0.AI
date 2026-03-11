# Example Workflows

## 1) Single Challenge

```bash
0x0 scan ./challenge
0x0 solve ./challenge --yes
0x0 replay <session-id>
0x0 writeup <session-id>
```

## 2) Batch Solve Across Directories

```bash
0x0 solve-all ./ctf-archive --yes --max-challenges 30
0x0 solve-all ../ --yes --max-challenges 80
```

## 3) Interactive Chat with Full Action Visibility

```bash
0x0 chat --show-actions --yes
```

Inside chat:
- `/research <query>`
- `/run <local command>`
- normal prompts to reason/code/explain
- mention flag format naturally (example: `flag prefix is HTB`) so autonomous mode can prioritize matching candidates

## 4) Web Challenge (Authorized Lab)

```bash
0x0 web map http://127.0.0.1:8080 --approve-network --approve-exec
0x0 web replay http://127.0.0.1:8080 --method POST --path /login --data 'u=test&p=test' --approve-network --approve-exec
```
