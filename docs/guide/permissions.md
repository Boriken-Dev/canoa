# Permissions

An agent asks before it does more than you allowed. {{ name }} shows the question as a card above the composer, and the session waits in the **Needs you** group of the list until you answer.

## Answering a card

The card says what the agent wants ("Allow Claude to edit calc.js?"), why it is asking when the agent says, and shows the change itself: the file and the lines that would go and come.

![A permission card showing the lines an edit would change](../assets/screens/window-permission-dark.png#only-dark)
![A permission card showing the lines an edit would change](../assets/screens/window-permission-light.png#only-light)

| You press | It means |
| --- | --- |
| **Yes** (or **Allow**) | Do this once. **Ctrl+Shift+A** answers it from the keyboard. |
| **Always allow**, and the arrow beside it | Remember the answer. The arrow lists narrower choices when the agent offers them. |
| **Deny** (**Ctrl+Shift+R**) | Refuse. The agent is told and carries on without it. |
| Typing in the composer and sending | Decline and redirect: your message tells the agent what to do instead. |

An **always** answer is kept by the agent, not by {{ name }}: the card says so, because {{ name }} cannot see or undo what the agent keeps. Settings, Permissions lists the rules {{ name }} itself remembers, each with **Revoke**.

![A permission card on a phone](../assets/screens/window-permission-phone-dark.png#only-dark)
![A permission card on a phone](../assets/screens/window-permission-phone-light.png#only-light)

!!! example "Worked example"
    You asked an agent to fix a failing test. It reads `src/calc.js` without asking (reading is allowed by default in the mode you chose), then stops with a card: "Allow Claude to edit calc.js?" showing `return a - b;` replaced by `return a + b;`.

    1. Read the two lines. They are the fix you expected, so press **Yes**.
    2. It then wants to edit `README.md`, which you did not ask for. Press **Deny**, or type "leave the README alone" and send.
    3. Later it asks to run `npm test`. Choose **Always allow** from the arrow's list for that command, so the same command is not asked about again.

## How much it asks: the mode

Each session has a mode, chosen from the **Mode** button under the composer or with `/mode`.

| Mode | What it does |
| --- | --- |
| **Manual** | Asks before each change or command. |
| **Accept edits** | Changes files without asking; commands still ask. |
| **Auto** | Lets the agent decide what needs asking. |
| **Plan** | Reads and plans; makes no changes. |
| **Bypass** | Asks for nothing. |

**Default** leaves the choice to the agent's own setting. Which modes exist, and what each does, comes from the agent: these are the ones the agents {{ name }} ships with offer.

![The mode menu of a new session](../assets/screens/window-modes-dark.png#only-dark)
![The mode menu of a new session](../assets/screens/window-modes-light.png#only-light)

## Settings, Permissions

**Settings, Permissions** sets the mode a new session starts in, whether **Bypass** answers with "allow once" only, a log of the rules made, used or revoked, and the list of remembered rules with their **Revoke** buttons.

![The Permissions page of Settings](../assets/screens/settings-permissions-dark.png#only-dark)
![The Permissions page of Settings](../assets/screens/settings-permissions-light.png#only-light)

The browser extension and the Office add-in have permissions of their own, for tabs and documents: see [the browser extension](browser-extension.md) and [the Office add-in](office-addin.md). They are separate on purpose: allowing an agent to edit a file never lets it read a tab, and the other way round.
