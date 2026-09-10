import { Text } from 'react-native';

export type GreetingProps = {
    name: string;
};

export function Greeting({ name }: GreetingProps): React.JSX.Element {
    return <Text>{name}</Text>;
}
